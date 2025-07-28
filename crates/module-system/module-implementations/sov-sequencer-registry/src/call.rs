use schemars::JsonSchema;
use sov_bank::{Amount, IntoPayable};
use sov_modules_api::macros::{config_value, serialize, UniversalWallet};
use sov_modules_api::registration_lib::RegistrationError;
use sov_modules_api::{Context, DaSpec, EventEmitter, ModuleInfo, Spec, TxState};

use crate::{
    gas_coins, BalanceState, CustomError, Event, KnownSequencer, SequencerRegistry,
    SequencerRegistryError,
};

/// This enumeration represents the available call messages for interacting with
/// the `sov-sequencer-registry` module.
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
pub enum CallMessage<S: Spec> {
    /// Add a new sequencer to the sequencer registry.
    Register {
        /// The Da address of the sequencer you're registering.
        da_address: <S::Da as DaSpec>::Address,
        /// The initial balance of the sequencer.
        amount: Amount,
    },
    /// Increases the balance of the sequencer, transferring the funds from the sequencer account
    /// to the rollup.
    Deposit {
        /// The DA address of the sequencer.
        da_address: <S::Da as DaSpec>::Address,
        /// The amount to increase.
        amount: Amount,
    },
    /// Initiate a withdrawal of a sequencer's balance.
    InitiateWithdrawal {
        /// The DA address of the sequencer you're removing.
        da_address: <S::Da as DaSpec>::Address,
    },
    /// Withdraw a sequencer's balance after waiting for the withdrawal period.
    Withdraw {
        /// The DA address of the sequencer you're removing.
        da_address: <S::Da as DaSpec>::Address,
    },
}

impl<S: Spec> SequencerRegistry<S> {
    /// Tries to register a sequencer by staking the provided amount of gas tokens.
    /// This method uses the context's sender as the sequencer's address.
    ///
    /// # Errors
    /// Will error
    ///
    /// - If the provided amount is below the minimum required to register a sequencer.
    /// - If the minimum bond is not set.
    /// - If the sender's account does not have enough funds to register itself as a sequencer.
    /// - If the sequencer is already registered.
    pub(crate) fn register<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        amount: Amount,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.register_staker(da_address, amount, context.sender().clone(), state)?;

        Ok(())
    }

    pub(crate) fn register_staker<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        amount: Amount,
        address: S::Address,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        if let Some(existing_sequencer) = self.known_sequencers.get(da_address, state)? {
            return Err(RegistrationError::AlreadyRegistered(
                existing_sequencer.address,
            ));
        }

        let Some(minimum_bond) = self.minimum_bond.get(state)? else {
            return Err(SequencerRegistryError::<S, ST>::NoMinimumBondSet);
        };

        if amount < minimum_bond {
            return Err(SequencerRegistryError::<S, ST>::InsufficientStakeAmount {
                address: address.clone(),
                bond_amount: amount,
                minimum_bond_amount: minimum_bond,
            });
        }

        self.bank
            .transfer_from(
                &address,
                self.id().clone().to_payable(),
                gas_coins(amount),
                state,
            )
            .map_err(
                |_| SequencerRegistryError::<S, ST>::InsufficientFundsToRegister {
                    address: address.clone(),
                    amount,
                },
            )?;
        let new_sequencer = KnownSequencer {
            address: address.clone(),
            balance: amount,
            balance_state: BalanceState::Active,
        };
        self.known_sequencers
            .set(da_address, &new_sequencer, state)?;

        self.emit_event(
            state,
            Event::<S>::Registered {
                sequencer: address,
                amount,
            },
        );
        Ok(())
    }

    pub(crate) fn deposit<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        amount: Amount,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.validate_sender(da_address, context.sender(), state)?;
        let Some(mut existing_sequencer) = self.known_sequencers.get(da_address, state)? else {
            return Err(RegistrationError::IsNotRegistered(da_address.clone()));
        };
        let address = existing_sequencer.address.clone();
        existing_sequencer.balance = existing_sequencer.balance.checked_add(amount).ok_or(
            SequencerRegistryError::<S, ST>::ToppingAccountMakesBalanceOverflow {
                address: address.clone(),
                existing_balance: existing_sequencer.balance,
                amount_to_add: amount,
            },
        )?;
        // Depositing re-activates the account if inactive.
        existing_sequencer.balance_state = BalanceState::Active;

        self.bank
            .transfer_from(
                &address,
                self.id().clone().to_payable(),
                gas_coins(amount),
                state,
            )
            .map_err(
                |_| SequencerRegistryError::<S, ST>::InsufficientFundsToTopUpAccount {
                    address: address.clone(),
                    amount_to_add: amount,
                },
            )?;

        self.known_sequencers
            .set(da_address, &existing_sequencer, state)?;

        self.emit_event(
            state,
            Event::<S>::Deposited {
                sequencer: address.clone(),
                amount: amount.0,
            },
        );

        Ok(())
    }

    /// Tries to remove a sequencer by unstaking the provided amount of gas tokens.
    /// This method uses the context's sender as the sequencer's address.
    ///
    /// # Errors
    /// Will error
    ///
    /// - If the sequencer is not registered.
    /// - If the sequencer tries to unregister itself during the execution of its own batch.
    /// - If the supplied `da_address` does not match the transaction sender.
    /// - If the module balance is not high enough to refund the sequencer's staked amount (this is a bug).
    pub(crate) fn initiate_withdrawal<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.validate_sender(da_address, context.sender(), state)?;
        let Some(mut existing_sequencer) = self.known_sequencers.get(da_address, state)? else {
            return Err(RegistrationError::IsNotRegistered(da_address.clone()));
        };

        if &existing_sequencer.address == context.sequencer() {
            return Err(RegistrationError::Custom(
                CustomError::CannotUnregisterDuringOwnBatch(da_address.clone()),
            ));
        }
        if existing_sequencer.balance_state != BalanceState::Active {
            return Err(RegistrationError::WithdrawalAlreadyPending(
                existing_sequencer.address,
            ));
        }

        // We force the sequencer to wait to withdraw until all of their pending blobs will have been selected for processing or dropped.
        // In the worst case, this could take up to `DEFERRED_SLOTS_COUNT` slots, so wait until the slot after that.
        existing_sequencer.balance_state = BalanceState::PendingWithdrawal {
            ready_at: state
                .current_visible_slot_number()
                .advance(config_value!("DEFERRED_SLOTS_COUNT") + 1),
        };
        self.known_sequencers
            .set(da_address, &existing_sequencer, state)?;

        self.emit_event(
            state,
            Event::<S>::InitiatedWithdrawal {
                sequencer: existing_sequencer.address.clone(),
            },
        );
        Ok(())
    }

    pub(crate) fn withdraw<ST: TxState<S>>(
        &mut self,
        da_address: &<S::Da as DaSpec>::Address,
        context: &Context<S>,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        self.validate_sender(da_address, context.sender(), state)?;
        let Some(existing_sequencer) = self.known_sequencers.get(da_address, state)? else {
            return Err(RegistrationError::IsNotRegistered(da_address.clone()));
        };
        let BalanceState::PendingWithdrawal { ready_at } = existing_sequencer.balance_state else {
            return Err(RegistrationError::Custom(
                CustomError::WithdrawalNotInitiated(da_address.clone()),
            ));
        };
        if ready_at > state.current_visible_slot_number() {
            return Err(RegistrationError::Custom(CustomError::WithdrawalNotReady {
                sequencer: da_address.clone(),
                current_visible_height: state.current_visible_slot_number(),
                ready_at,
            }));
        }
        self.known_sequencers.delete(da_address, state)?;
        self.bank
            .transfer_from(
                self.id().clone().to_payable(),
                &existing_sequencer.address,
                gas_coins(existing_sequencer.balance),
                state,
            )
            .expect("Failed to withdraw a sequencer balance. This indicates a bug in accounting!");

        self.emit_event(
            state,
            Event::<S>::Withdrew {
                sequencer: existing_sequencer.address.clone(),
                amount_withdrawn: existing_sequencer.balance,
            },
        );

        Ok(())
    }

    fn validate_sender<ST: TxState<S>>(
        &self,
        da_address: &<S::Da as DaSpec>::Address,
        sender: &S::Address,
        state: &mut ST,
    ) -> Result<(), SequencerRegistryError<S, ST>> {
        let belongs_to = self
            .known_sequencers
            .get_or_err(da_address, state)?
            .map_err(|_| RegistrationError::IsNotRegistered(da_address.clone()))?
            .address;

        if sender != &belongs_to {
            return Err(RegistrationError::Custom(
                CustomError::SuppliedAddressDoesNotMatchTxSender {
                    parameter: belongs_to,
                    sender: sender.clone(),
                },
            ));
        }

        Ok(())
    }
}
