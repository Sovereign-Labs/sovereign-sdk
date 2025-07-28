//! The `sov-sequencer-registry` module is responsible for sequencer
//! registration, slashing, and rewards. At the moment, only a centralized
//! sequencer is supported. The sequencer's address and bond are registered
//! during the rollup deployment.
//!
#![deny(missing_docs)]
mod call;
mod capabilities;
mod event;
mod genesis;
mod registration;

use std::convert::Infallible;

use anyhow::bail;
use borsh::{BorshDeserialize, BorshSerialize};
pub use call::*;
pub use event::Event;
pub use genesis::*;
use registration::gas_coins;
use serde::{Deserialize, Serialize};
use sov_bank::derived_holder::DerivedHolder;
use sov_bank::{Amount, IntoPayable};
use sov_modules_api::capabilities::AllowedSequencer;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::registration_lib::RegistrationError;
#[cfg(feature = "native")]
use sov_modules_api::ApiStateAccessor;
use sov_modules_api::{
    BasicAddress, Context, DaSpec, GenesisState, InfallibleStateAccessor, KernelStateAccessor,
    KernelStateMap, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, StateAccessor, StateReader,
    StateValue, StateWriter, TxState, VisibleSlotNumber,
};
use sov_state::{Kernel, User};
use thiserror::Error;

/// Errors that can be raised by the [`SequencerRegistry`] module during hooks execution.
#[derive(
    Debug, Clone, Error, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AllowedSequencerError {
    /// The sequencer is not registered.
    #[error("The sequencer is not registered.")]
    NotRegistered,
    /// The sequencer is known but not active.
    #[error("The sequencer is known but not active.")]
    NotActive,
}

/// An known sequencer for a rollup.
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, Eq, PartialEq)]
#[serde(bound = "S::Address: serde::Serialize + serde::de::DeserializeOwned")]
pub struct KnownSequencer<S: Spec> {
    /// The rollup address of the sequencer.
    pub address: S::Address,
    /// The staked balance of the sequencer.
    pub balance: Amount,
    /// The balance state of the sequencer.
    pub balance_state: BalanceState,
}

impl<S: Spec> TryFrom<KnownSequencer<S>> for AllowedSequencer<S> {
    type Error = anyhow::Error;

    fn try_from(value: KnownSequencer<S>) -> Result<Self, Self::Error> {
        if value.balance_state.is_active() {
            Ok(Self {
                address: value.address,
                balance: value.balance,
            })
        } else {
            Err(anyhow::anyhow!("Sequencer is not active"))
        }
    }
}

/// The status of the sequencer's balance.
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, Eq, PartialEq)]
pub enum BalanceState {
    /// The sequencer has enough balance to submit and process batches.
    Active,
    /// The sequencer has insufficient balance to submit and process batches.
    PendingWithdrawal {
        /// The slot number at which the sequencer will be able to withdraw.
        ready_at: VisibleSlotNumber,
    },
}

impl BalanceState {
    /// Returns true if the sequencer is active.
    pub fn is_active(&self) -> bool {
        matches!(self, BalanceState::Active)
    }

    /// Returns true if the sequencer is pending withdrawal.
    pub fn is_pending_withdrawal(&self) -> bool {
        matches!(self, BalanceState::PendingWithdrawal { .. })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
/// Reason why sequencer was slashed.
pub enum SlashingReason {
    /// This status indicates problem with batch deserialization.
    InvalidBatchEncoding,
    /// Stateless verification failed, for example deserialized transactions have invalid signatures.
    StatelessVerificationFailed,
    /// This status indicates problem with transaction deserialization.
    InvalidTransactionEncoding,
}

/// The `sov-sequencer-registry` module `struct`.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct SequencerRegistry<S: Spec> {
    /// The ID of the `sov_sequencer_registry` module.
    #[id]
    pub(crate) id: ModuleId,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<S>,

    /// Only batches from sequencers from this list are going to be processed.
    /// We need to map the DA address to the rollup address because the sequencer interacts with the rollup
    /// through the DA layer.
    #[state]
    pub(crate) known_sequencers: KernelStateMap<<S::Da as DaSpec>::Address, KnownSequencer<S>>,

    /// Optional preferred sequencer.
    /// If set, batches from this sequencer will be processed first in block,
    /// So this sequencer can guarantee soft confirmation time for transactions
    #[state]
    pub(crate) preferred_sequencer: StateValue<<S::Da as DaSpec>::Address>,

    /// Minimum bond for the sequencer to be registered.
    #[state]
    pub(crate) minimum_bond: StateValue<Amount>,
}

/// A special error type that can be raised when calling a method from the sequencer registry
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CustomError<RollupAddress: BasicAddress, DaAddress: BasicAddress> {
    /// The sequencer tried to unregister itself during the execution of its own batch.
    #[error("Sequencers may not unregister during execution of their own batch")]
    CannotUnregisterDuringOwnBatch(DaAddress),

    /// The sequencer tried to withdraw without first initiating a withdrawal and waiting for the withdrawal to be ready.
    #[error("Sequencers may not withdraw without first initiating a withdrawal and waiting for the withdrawal to be ready")]
    WithdrawalNotInitiated(DaAddress),

    /// The sequencer tried to withdraw before the withdrawal was ready.
    #[error("Sequencer {sequencer} may not withdraw before the withdrawal is ready. Current visible height: {current_visible_height}, Ready at: {ready_at}")]
    WithdrawalNotReady {
        /// The address of the sequencer that tried to withdraw.
        sequencer: DaAddress,
        /// The current visible height of the chain.
        current_visible_height: VisibleSlotNumber,
        /// The slot number at which the withdrawal will be ready.
        ready_at: VisibleSlotNumber,
    },

    #[error("The address provided as a parameter to the `exit` method does not match the transaction sender")]
    /// The address provided as a parameter to the `exit` method does not match the transaction sender.
    SuppliedAddressDoesNotMatchTxSender {
        /// The address provided as a parameter to the `exit` method.
        parameter: RollupAddress,
        /// The address of the transaction sender.
        sender: RollupAddress,
    },
}

/// The different errors that can be raised by the sequencer registry
#[allow(type_alias_bounds)]
pub type SequencerRegistryError<S: Spec, ST: StateAccessor> = RegistrationError<
    S::Address,
    <S::Da as DaSpec>::Address,
    <ST as StateReader<User>>::Error,
    CustomError<S::Address, <S::Da as DaSpec>::Address>,
>;

impl<S: Spec> Module for SequencerRegistry<S> {
    type Spec = S;

    type Config = SequencerRegistryConfig<S>;

    type CallMessage = CallMessage<S>;

    type Event = Event<S>;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.init_module(config, state)
    }

    fn call(
        &mut self,
        message: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match message {
            CallMessage::Register { da_address, amount } => {
                Ok(self.register(&da_address, amount, context, state)?)
            }

            CallMessage::Deposit { da_address, amount } => {
                Ok(self.deposit(&da_address, amount, context, state)?)
            }

            CallMessage::InitiateWithdrawal { da_address } => {
                Ok(self.initiate_withdrawal(&da_address, context, state)?)
            }

            CallMessage::Withdraw { da_address } => {
                Ok(self.withdraw(&da_address, context, state)?)
            }
        }
    }
}

impl<S: Spec> SequencerRegistry<S> {
    /// Returns the preferred sequencer, or [`None`] it wasn't set.
    ///
    /// Read about [`SequencerConfig::is_preferred_sequencer`] to learn about
    /// preferred sequencers.
    #[allow(clippy::type_complexity)]
    pub fn get_preferred_sequencer<
        Reader: StateReader<Kernel, Error = E> + StateReader<User, Error = E>,
        E,
    >(
        &self,
        state: &mut Reader,
    ) -> Result<Option<(<S::Da as DaSpec>::Address, S::Address)>, E> {
        if let Some(da_addr) = self.preferred_sequencer.get(state)? {
            // If the preferred sequencer address is set but they're not currently authorized, act like there is no preferred sequencer
            Ok(self
                .known_sequencers
                .get(&da_addr, state)?
                .map(|seq| (da_addr, seq.address)))
        } else {
            Ok(None)
        }
    }

    /// Retrieves the escrowed funds and transfers the amount needed for pre-execution checks to the bank module.
    /// The remaining amount is transferred back to the selected recipient.
    pub fn retrieve_funds_from_escrow(
        &mut self,
        holder: &DerivedHolder,
        recipient: &S::Address,
        tokens_needed_for_pre_exec_checks: Amount,
        state: &mut impl InfallibleStateAccessor,
    ) -> Result<(), anyhow::Error> {
        let Some(reserved_balance) = self
            .bank
            .get_balance_of(holder.to_payable(), sov_bank::config_gas_token_id(), state)
            .unwrap_infallible()
        else {
            bail!("No reserved balance found for holder {}", holder);
        };

        let Some(amount_to_refund) =
            reserved_balance.checked_sub(tokens_needed_for_pre_exec_checks)
        else {
            bail!(
                "Not enough reserved balance to refund {} from holder {}. Needed {}, but only {} was available",
                recipient,
                holder,
                tokens_needed_for_pre_exec_checks,
                reserved_balance
            );
        };

        self.bank.transfer_from(
            holder.to_payable(),
            recipient,
            gas_coins(amount_to_refund),
            state,
        )?;
        self.bank
            .transfer_from(
                holder.to_payable(),
                self.bank.id().clone().to_payable(),
                gas_coins(tokens_needed_for_pre_exec_checks),
                state,
            )
            .expect("Failed to transfer a valid balance. This should never happen.");
        Ok(())
    }

    /// Refunds the holder for the unneeded reserved gas.
    pub fn refund_all_reserved_gas(
        &mut self,
        holder: &DerivedHolder,
        recipient: &S::Address,
        state: &mut impl InfallibleStateAccessor,
    ) {
        let Some(reserved_balance) = self
            .bank
            .get_balance_of(holder.to_payable(), sov_bank::config_gas_token_id(), state)
            .unwrap_infallible()
        else {
            // If the holder has no reserved balance, we're done.
            return;
        };
        self.bank
            .transfer_from(
                holder.to_payable(),
                recipient,
                gas_coins(reserved_balance),
                state,
            )
            .expect("Failed to transfer a valid balance. This should never happen.");
    }

    /// Checks whether `sender` is a registered sequencer with enough staked amount.
    /// If so, returns the allowed sequencer in a [`AllowedSequencer`] object.
    /// Otherwise, returns a [`AllowedSequencerError`].
    pub fn is_sender_allowed(
        &self,
        sender: &<S::Da as DaSpec>::Address,
        state: &mut impl StateReader<Kernel, Error = Infallible>,
    ) -> Result<AllowedSequencer<S>, AllowedSequencerError> {
        let sequencer = self.is_sender_known(sender, state)?;
        if let Ok(sequencer) = sequencer.try_into() {
            return Ok(sequencer);
        }
        Err(AllowedSequencerError::NotActive)
    }

    /// Checks whether `sender` is a registered sequencer. Note that this method does not check if the sequencer is currenltly active!
    /// If so, returns the known sequencer in a [`KnownSequencer`] object.
    /// Otherwise, returns a [`AllowedSequencerError`].
    pub fn is_sender_known(
        &self,
        sender: &<S::Da as DaSpec>::Address,
        state: &mut impl StateReader<Kernel, Error = Infallible>,
    ) -> Result<KnownSequencer<S>, AllowedSequencerError> {
        if let Some(sequencer) = self.known_sequencers.get(sender, state).unwrap_infallible() {
            return Ok(sequencer);
        }

        Err(AllowedSequencerError::NotRegistered)
    }

    /// Returns the balance of the provided sender, if present.
    pub fn get_sender_balance(
        &self,
        sender: &<S::Da as DaSpec>::Address,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> Option<Amount> {
        self.known_sequencers
            .get(sender, state)
            .unwrap_infallible()
            .map(|s| s.balance)
    }

    /// Returns the balance of the provided sender, if present.
    ///
    /// This method is only available in via API, since sequencer balances depend directly on the state of the DA layer.
    /// If you need access to balances on-chain, use the `get_sender_balance` method with a `KernelStateAccessor`.
    #[cfg(feature = "native")]
    pub fn get_sender_balance_via_api(
        &self,
        sender: &<S::Da as DaSpec>::Address,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<Amount> {
        self.known_sequencers
            .get(sender, state)
            .unwrap_infallible()
            .map(|s| s.balance)
    }

    /// Returns the rollup address of the sequencer with the given DA address.
    #[cfg(feature = "native")]
    pub fn get_sequencer_address(
        &self,
        da_address: <S::Da as DaSpec>::Address,
        state_accessor: &mut ApiStateAccessor<S>,
    ) -> Result<Option<S::Address>, Infallible> {
        Ok(self
            .known_sequencers
            .get(&da_address, state_accessor)
            .unwrap_infallible()
            .map(|s| s.address))
    }

    /// Slash the sequencer with the given address.
    pub fn slash_sequencer<
        Accessor: StateWriter<Kernel, Error = Infallible>
            + StateReader<User, Error = Infallible>
            + StateWriter<User, Error = Infallible>,
    >(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        state: &mut Accessor,
    ) {
        self.known_sequencers
            .delete(da_address, state)
            .unwrap_infallible();

        if let Some(preferred_sequencer) = self.preferred_sequencer.get(state).unwrap_infallible() {
            if da_address == &preferred_sequencer {
                self.preferred_sequencer.delete(state).unwrap_infallible();
            }
        }
    }

    /// Check if the provided `Da::Address` belongs to a registered sequencer.
    #[cfg(feature = "native")]
    pub fn is_registered_sequencer(
        &self,
        da_address: &<S::Da as DaSpec>::Address,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<bool, Infallible> {
        Ok(self.known_sequencers.get(da_address, state)?.is_some())
    }
}
