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
    /// Name of the token.
    pub(crate) name: String,
    /// Total supply of the coins.
    pub(crate) total_supply: u64,
    /// The special address can be used as burn address or to store temporarily locked coins.
    pub(crate) special_address: C::Address,
    /// Mapping from user address to user balance.
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
            // TODO: Add `from` address to the message (we need pretty print for Address first)
            None => bail!("Insufficient funds"),
        };

        // We can't overflow here because the sum must be smaller than `total_supply` which is u64.
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
        self.transfer(from, &self.special_address, amount, working_set)
    }
}
