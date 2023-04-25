use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::call::prefix_from_address;

pub type Amount = u64;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
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
        if from == to {
            return Ok(CallResponse::default());
        }
        let from_balance = self.balances.get_or_err(from, working_set)?;

        let from_balance = match from_balance.checked_sub(amount) {
            Some(from_balance) => from_balance,
            // TODO: Add `from` address to the message (we need pretty print for Address first)
            None => bail!("Insufficient funds"),
        };

        // We can't overflow here because the sum must be smaller or eq to `total_supply` which is u64.
        let to_balance = self.balances.get(to, working_set).unwrap_or_default() + amount;

        self.balances.set(from, from_balance, working_set);
        self.balances.set(to, to_balance, working_set);

        Ok(CallResponse::default())
    }

    pub(crate) fn burn(
        &mut self,
        from: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let balance = self.balances.get_or_err(from, working_set)?;
        // TODO: Should we burn as much as we can or error if it was more than balance?
        let new_balance = match balance.checked_sub(amount) {
            Some(from_balance) => from_balance,
            // TODO: Add `from` address to the message (we need pretty print for Address first)
            None => bail!("Insufficient funds"),
        };
        self.balances.set(from, new_balance, working_set);
        self.total_supply -= amount;
        Ok(CallResponse::default())
    }

    pub(crate) fn create(
        token_name: &str,
        address_and_balances: &[(C::Address, u64)],
        sender: &[u8],
        salt: u64,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(C::Address, Self)> {
        let token_address = super::create_token_address::<C>(token_name, sender, salt);

        let token_prefix = prefix_from_address::<C>(&token_address);
        let balances = sov_state::StateMap::new(token_prefix);

        let mut total_supply: Option<u64> = Some(0);

        for (address, balance) in address_and_balances.iter() {
            balances.set(address, *balance, working_set);
            total_supply = total_supply.and_then(|ts| ts.checked_add(*balance));
        }

        let total_supply = match total_supply {
            Some(total_supply) => total_supply,
            None => bail!("Total supply overflow"),
        };

        let token = Token::<C> {
            name: token_name.to_owned(),
            total_supply,
            balances,
        };

        Ok((token_address, token))
    }
}
