pub(crate) mod attester;
pub(crate) mod challenger;
use core::result::Result::Ok;

use sov_bank::{config_gas_token_id, Amount, Coins, IntoPayable};
use sov_modules_api::registration_lib::{RegistrationError, StakeRegistration};
use sov_modules_api::{
    BasicAddress, Gas, GetGasPrice, ModuleId, Spec, StateAccessor, StateMap, StateReader,
    StateValue, TxState,
};
use sov_state::User;
use thiserror::Error;

use crate::AttesterIncentives;

/// Custom errors for the attester incentives module
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CustomError<RollupAddress: BasicAddress> {
    #[error("Attester is unbonding")]
    /// The attester is in the first unbonding phase.
    AttesterIsUnbonding(RollupAddress),

    #[error("The first phase of unbonding has not been finalized")]
    /// The attester is trying to finish the two-phase unbonding too soon.
    UnbondingNotFinalized(RollupAddress),

    #[error("User is not trying to unbond at the time of the transaction")]
    /// User is not trying to unbond at the time of the transaction.
    AttesterIsNotUnbonding(RollupAddress),
}

#[allow(type_alias_bounds)]
type AttesterRegistryError<S: Spec, ST: StateAccessor> = RegistrationError<
    S::Address,
    S::Address,
    <ST as StateReader<User>>::Error,
    CustomError<S::Address>,
>;

struct Staker<'a, S: Spec> {
    bonded_stakers: &'a mut StateMap<S::Address, Amount>,
    minimum_bond: &'a mut StateValue<S::Gas>,
    bank: &'a mut sov_bank::Bank<S>,
    id: &'a ModuleId,
}

impl<'a, S: Spec> Staker<'a, S> {
    fn new_challenger(attester_incentives: &'a mut AttesterIncentives<S>) -> Self {
        Self {
            bonded_stakers: &mut attester_incentives.bonded_challengers,
            minimum_bond: &mut attester_incentives.minimum_challenger_bond,
            bank: &mut attester_incentives.bank,
            id: &attester_incentives.id,
        }
    }

    fn new_attester(attester_incentives: &'a mut AttesterIncentives<S>) -> Self {
        Self {
            bonded_stakers: &mut attester_incentives.bonded_attesters,
            minimum_bond: &mut attester_incentives.minimum_attester_bond,
            bank: &mut attester_incentives.bank,
            id: &attester_incentives.id,
        }
    }
}

impl<S> StakeRegistration for Staker<'_, S>
where
    S: Spec,
{
    type PrimaryAddress = S::Address;

    type RollupAddress = S::Address;

    type CustomError = CustomError<S::Address>;

    type Spec = S;

    fn get_minimum_bond<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &self,
        state: &mut ST,
    ) -> Result<Option<Amount>, <ST as sov_modules_api::StateWriter<sov_state::User>>::Error> {
        self.minimum_bond
            .get(state)
            .map(|maybe_bond| maybe_bond.map(|bond| bond.value(state.gas_price())))
    }

    fn get_allowed_staker<ST: StateAccessor>(
        &self,
        address: &Self::PrimaryAddress,
        state: &mut ST,
    ) -> Result<
        Option<(Self::RollupAddress, Amount)>,
        <ST as sov_modules_api::StateWriter<sov_state::User>>::Error,
    > {
        self.bonded_stakers
            .get(address, state)
            .map(|opt| opt.map(|bond| (address.clone(), bond)))
    }

    fn set_allowed_staker<ST: StateAccessor>(
        &mut self,
        _primary_address: &Self::PrimaryAddress,
        rollup_address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> Result<(), <ST as sov_modules_api::StateWriter<sov_state::User>>::Error> {
        self.bonded_stakers.set(rollup_address, &amount, state)?;
        Ok(())
    }

    fn transfer_bond_from_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> anyhow::Result<()> {
        self.bank
            .transfer_from(address, self.id.to_payable(), gas_coins(amount), state)?;
        Ok(())
    }

    fn transfer_bond_to_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> anyhow::Result<()> {
        self.bank
            .transfer_from(self.id.to_payable(), address, gas_coins(amount), state)?;
        Ok(())
    }

    fn delete_allowed_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::PrimaryAddress,
        state: &mut ST,
    ) -> Result<(), <ST as sov_modules_api::StateWriter<sov_state::User>>::Error> {
        self.bonded_stakers.delete(address, state)
    }
}

pub(crate) fn gas_coins(amount: Amount) -> Coins {
    Coins {
        amount,
        token_id: config_gas_token_id(),
    }
}
