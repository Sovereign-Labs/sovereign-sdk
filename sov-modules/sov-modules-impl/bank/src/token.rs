use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

pub type Amount = u64;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct Coins<Address: sov_modules_api::AddressTrait> {
    pub amount: Amount,
    pub token_address: Address,
}

/// This struct represents a token in the bank module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub(crate) struct Token<C: sov_modules_api::Context> {
    pub(crate) name: String,
    pub(crate) total_supply: u64,
    pub(crate) burn_address: C::Address,
    pub(crate) balances: sov_state::StateMap<C::Address, Amount>,
}

impl<C: sov_modules_api::Context> Token<C> {
    pub(crate) fn transfer(
        &self,
        from: &C::Address,
        to: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let from_balance = self.balances.get_or_err(from, working_set)?;

        let from_balance = match from_balance.checked_sub(amount) {
            Some(from_balance) => from_balance,
            None => bail!("todo"),
        };

        let to_balance = self.balances.get(to, working_set).unwrap_or_default() + amount;

        self.balances.set(from, from_balance, working_set);
        self.balances.set(to, to_balance, working_set);

        Ok(CallResponse::default())
    }

    pub(crate) fn burn(
        &self,
        from: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.transfer(from, &self.burn_address, amount, working_set)
    }
}
