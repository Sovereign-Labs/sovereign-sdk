use anyhow::Result;
use sov_bank::{config_gas_token_id, Amount, Coins, IntoPayable};
use sov_modules_api::registration_lib::StakeRegistration;
use sov_modules_api::{Gas, GetGasPrice, ModuleInfo, Spec, StateAccessor, TxState};
use sov_state::User;

use crate::{CustomError, ProverIncentives};

impl<S: Spec> StakeRegistration for ProverIncentives<S> {
    type Spec = S;

    type PrimaryAddress = S::Address;

    type RollupAddress = S::Address;

    type CustomError = CustomError;

    fn get_minimum_bond<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &self,
        state: &mut ST,
    ) -> Result<Option<Amount>, <ST as sov_modules_api::StateWriter<User>>::Error> {
        self.minimum_bond.get(state).map(|maybe_minimum_bond| {
            maybe_minimum_bond.map(|minimum_bond| minimum_bond.value(state.gas_price()))
        })
    }

    fn get_allowed_staker<ST: StateAccessor>(
        &self,
        address: &Self::PrimaryAddress,
        state: &mut ST,
    ) -> std::result::Result<
        Option<(Self::RollupAddress, Amount)>,
        <ST as sov_modules_api::StateWriter<User>>::Error,
    > {
        self.bonded_provers
            .get(address, state)
            .map(|opt| opt.map(|b| (address.clone(), b)))
    }

    fn set_allowed_staker<ST: StateAccessor>(
        &mut self,
        primary_address: &Self::PrimaryAddress,
        _rollup_address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> Result<(), <ST as sov_modules_api::StateWriter<User>>::Error> {
        self.bonded_provers.set(primary_address, &amount, state)?;
        Ok(())
    }

    fn transfer_bond_from_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> anyhow::Result<()> {
        self.bank.transfer_from(
            address,
            self.id().clone().to_payable(),
            gas_coins(amount),
            state,
        )?;
        Ok(())
    }

    fn transfer_bond_to_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::RollupAddress,
        amount: Amount,
        state: &mut ST,
    ) -> anyhow::Result<()> {
        self.bank.transfer_from(
            self.id().clone().to_payable(),
            address,
            gas_coins(amount),
            state,
        )?;
        Ok(())
    }

    fn delete_allowed_staker<ST: StateAccessor>(
        &mut self,
        address: &Self::PrimaryAddress,
        state: &mut ST,
    ) -> Result<(), <ST as sov_modules_api::StateWriter<User>>::Error> {
        self.bonded_provers.delete(address, state)?;
        Ok(())
    }
}

fn gas_coins(amount: Amount) -> Coins {
    Coins {
        amount,
        token_id: config_gas_token_id(),
    }
}
