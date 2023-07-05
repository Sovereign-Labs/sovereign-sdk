use std::collections::HashSet;

use anyhow::{bail, Result};
use sov_state::{Prefix, WorkingSet};

use crate::call::prefix_from_address_with_parent;

pub type Amount = u64;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct Coins<C: sov_modules_api::Context> {
    pub amount: Amount,
    pub token_address: C::Address,
}

/// This struct represents a token in the sov-bank module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub(crate) struct Token<C: sov_modules_api::Context> {
    /// Name of the token.
    pub(crate) name: String,
    /// Total supply of the coins.
    pub(crate) total_supply: u64,
    /// Mapping from user address to user balance.
    pub(crate) balances: sov_state::StateMap<C::Address, Amount>,

    /// Vector containing the authorized minters
    /// Empty vector indicates that the token supply is frozen
    /// Non empty vector indicates members of the vector can mint.
    /// Freezing a token requires emptying the vector
    /// NOTE: This is explicit so if a creator doesn't add themselves, then they can't mint
    pub(crate) authorized_minters: Vec<C::Address>,
}

impl<C: sov_modules_api::Context> Token<C> {
    pub(crate) fn transfer(
        &self,
        from: &C::Address,
        to: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let from_balance = self.check_balance(from, amount, working_set)?;

        // We can't overflow here because the sum must be smaller or eq to `total_supply` which is u64.
        let to_balance = self.balances.get(to, working_set).unwrap_or_default() + amount;

        self.balances.set(from, &from_balance, working_set);
        self.balances.set(to, &to_balance, working_set);

        Ok(())
    }

    pub(crate) fn burn(
        &mut self,
        from: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let new_balance = self.check_balance(from, amount, working_set)?;
        self.balances.set(from, &new_balance, working_set);

        Ok(())
    }

    /// Freezing a token requires emptying the authorized_minter vector
    /// authorized_minter: Vec<Address> is used to determine if the token is frozen or not
    /// If the vector is empty when the function is called, this means the token is already frozen
    pub(crate) fn freeze(&mut self, sender: &C::Address) -> Result<()> {
        if self.authorized_minters.is_empty() {
            bail!("Token is already frozen")
        }
        self.is_authorized_minter(sender)?;
        self.authorized_minters = vec![];
        Ok(())
    }

    pub(crate) fn mint(
        &mut self,
        sender: &C::Address,
        minter_address: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        if self.authorized_minters.is_empty() {
            bail!("Attempt to mint frozen token")
        }
        self.is_authorized_minter(sender)?;
        let to_balance: Amount = self
            .balances
            .get(minter_address, working_set)
            .unwrap_or_default()
            .checked_add(amount)
            .ok_or(anyhow::Error::msg(
                "Account Balance overflow in the mint method of bank module",
            ))?;

        self.balances.set(minter_address, &to_balance, working_set);
        self.total_supply = self
            .total_supply
            .checked_add(amount)
            .ok_or(anyhow::Error::msg(
                "Total Supply overflow in the mint method of bank module",
            ))?;
        Ok(())
    }

    fn is_authorized_minter(&self, sender: &C::Address) -> Result<()> {
        if !self.authorized_minters.contains(sender) {
            bail!("Sender {} is not an authorized minter", sender)
        }
        Ok(())
    }

    // Check that amount can be deducted from address
    // Returns new balance after subtraction.
    fn check_balance(
        &self,
        from: &C::Address,
        amount: Amount,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<Amount> {
        let balance = self.balances.get_or_err(from, working_set)?;
        let new_balance = match balance.checked_sub(amount) {
            Some(from_balance) => from_balance,
            None => bail!("Insufficient funds for {}", from),
        };
        Ok(new_balance)
    }

    pub(crate) fn create(
        token_name: &str,
        address_and_balances: &[(C::Address, u64)],
        authorized_minters: Vec<C::Address>,
        sender: &[u8],
        salt: u64,
        parent_prefix: &Prefix,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(C::Address, Self)> {
        let token_address = super::create_token_address::<C>(token_name, sender, salt);
        let token_prefix = prefix_from_address_with_parent::<C>(parent_prefix, &token_address);
        let balances = sov_state::StateMap::new(token_prefix);

        let mut total_supply: Option<u64> = Some(0);
        for (address, balance) in address_and_balances.iter() {
            balances.set(address, balance, working_set);
            total_supply = total_supply.and_then(|ts| ts.checked_add(*balance));
        }

        let total_supply = match total_supply {
            Some(total_supply) => total_supply,
            None => bail!("Total supply overflow"),
        };

        let mut indices = HashSet::new();
        let mut auth_minter_list = Vec::new();

        for (i, item) in authorized_minters.iter().enumerate() {
            if indices.insert(item.as_ref()) {
                auth_minter_list.push(authorized_minters[i].clone());
            }
        }

        let token = Token::<C> {
            name: token_name.to_owned(),
            total_supply,
            balances,
            authorized_minters: auth_minter_list,
        };

        Ok((token_address, token))
    }
}
