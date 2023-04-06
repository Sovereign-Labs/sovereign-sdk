use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

pub type Amount = u64;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub struct Coins<Address: sov_modules_api::AddressTrait> {
    pub(crate) amount: Amount,
    pub(crate) token_address: Address,
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct Token<C: sov_modules_api::Context> {
    name: String,
    total_supply: u64,
    burn_address: C::Address,
    balances: sov_state::StateMap<C::Address, Amount>,
}

impl<C: sov_modules_api::Context> Token<C> {
    pub fn transfer(
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

    pub fn burn(
        &self,
        from: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.transfer(from, &self.burn_address, amount, working_set)
    }
}
