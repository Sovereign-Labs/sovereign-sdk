use std::fmt::Debug;

use anyhow::Result;
use schemars::JsonSchema;
use sov_bank::BurnRate;
use sov_modules_api::macros::{config_value, serialize, UniversalWallet};
use sov_modules_api::registration_lib::{RegistrationError, StakeRegistration};
use sov_modules_api::{Amount, EventEmitter, Spec, StateAccessor, StateReader, TxState};
use sov_state::User;
use thiserror::Error;

use crate::{Event, ProverIncentives};

/// This enumeration represents the available call messages for interacting with the `ExampleModule` module.
#[derive(Clone, Debug, PartialEq, Eq, UniversalWallet, JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
// TODO: allow call messages to borrow data
//     https://github.com/Sovereign-Labs/sovereign-sdk/issues/274
pub enum CallMessage {
    /// Add a new prover as a bonded prover.
    Register(Amount),
    /// Increases the balance of the prover, transferring the funds from the prover account
    /// to the rollup.
    Deposit(Amount),
    /// Unbonds the prover.
    Exit,
}

/// The prover incentives module does not have a custom error. Therefore, this enum is non instantiable type
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CustomError {}

#[allow(type_alias_bounds)]
type ProverRegistryError<S: Spec, ST: StateAccessor> =
    RegistrationError<S::Address, S::Address, <ST as StateReader<User>>::Error, CustomError>;

impl<S: Spec> ProverIncentives<S> {
    /// The burn rate of the reward price for the provers.
    /// The burn rate is a percentage of the base fee that is burned - this prevents provers from proving empty blocks.
    pub fn burn_rate(&self) -> BurnRate {
        BurnRate::new_unchecked(config_value!("PERCENT_BASE_FEE_TO_BURN"))
    }

    /// Try to bond the requested amount of coins from context.sender()
    pub(crate) fn register<ST: TxState<S>>(
        &mut self,
        bond_amount: Amount,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), ProverRegistryError<S, ST>> {
        self.register_staker(prover_address, prover_address, bond_amount, state)?;
        self.emit_event(
            state,
            Event::<S>::Registered {
                prover: prover_address.clone(),
                amount: bond_amount,
            },
        );

        Ok(())
    }

    /// Increases the balance of the provided sender, updating the state of the bonded provers.
    pub(crate) fn deposit<ST: TxState<S>>(
        &mut self,
        amount: Amount,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), ProverRegistryError<S, ST>> {
        self.deposit_funds(prover_address, amount, state)?;
        self.emit_event(
            state,
            Event::<S>::Deposited {
                prover: prover_address.clone(),
                deposit: amount,
            },
        );

        Ok(())
    }

    /// Try to unbond the requested amount of coins with context.sender() as the beneficiary.
    pub(crate) fn exit<ST: TxState<S>>(
        &mut self,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), ProverRegistryError<S, ST>> {
        let amount_withdrawn = self.exit_staker(prover_address, state)?;

        self.emit_event(
            state,
            Event::<S>::Exited {
                prover: prover_address.clone(),
                amount_withdrawn,
            },
        );

        Ok(())
    }
}
