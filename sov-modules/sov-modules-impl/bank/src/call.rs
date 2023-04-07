use crate::{Amount, Bank, Coins, Token};
use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

/// This enumeration represents the available call messages for interacting with the bank module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum CallMessage<C: sov_modules_api::Context> {
    /// Creates a new token with the specified name and initial balance.
    CreateToken {
        /// salt: a random value use to create a unique token address.
        salt: u64,
        /// token_name: the name of the new token.
        token_name: String,
        /// initial_balance: the initial balance of the new token.
        initial_balance: Amount,
        /// minter_address: the address of the account that minted new tokens.
        minter_address: C::Address,
    },

    /// Transfers a specified amount of tokens to the specified address.
    Transfer {
        /// to: the address to which the tokens will be transferred.
        to: C::Address,
        /// coins: the amount of tokens to transfer.
        coins: Coins<C::Address>,
    },

    /// Burns a specified amount of tokens.
    Burn {
        /// coins: the amount of tokens to burn.
        coins: Coins<C::Address>,
    },
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn create_token(
        &self,
        token_name: String,
        salt: u64,
        initial_balance: Amount,
        minter_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token_address = super::create_token_address::<C>(&token_name, context.sender(), salt);

        match self.tokens.get(&token_address, working_set) {
            Some(_) => bail!("Token address already exists"),

            None => {
                let token_prefix = self.prefix_from_address(&token_address);

                // Create balances map and initialize minter balance.
                let balances = sov_state::StateMap::new(token_prefix);
                balances.set(&minter_address, initial_balance, working_set);

                let token = Token::<C> {
                    name: token_name,
                    total_supply: initial_balance,
                    balances,
                };

                self.tokens.set(&token_address, token, working_set);
            }
        };

        Ok(CallResponse::default())
    }

    pub(crate) fn transfer(
        &self,
        to: C::Address,
        coins: Coins<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token = self.tokens.get_or_err(&coins.token_address, working_set)?;
        token.transfer(context.sender(), &to, coins.amount, working_set)
    }

    pub(crate) fn burn(
        &self,
        coins: Coins<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token = self.tokens.get_or_err(&coins.token_address, working_set)?;
        token.burn(
            context.sender(),
            &coins.token_address,
            coins.amount,
            working_set,
        )
    }
}

impl<C: sov_modules_api::Context> Bank<C> {
    fn prefix_from_address(&self, token_address: &C::Address) -> sov_state::Prefix {
        sov_state::Prefix::new(token_address.as_ref().to_vec())
    }
}
