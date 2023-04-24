use crate::{Amount, Bank, Coins, Token};
use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

/// This enumeration represents the available call messages for interacting with the bank module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage<C: sov_modules_api::Context> {
    /// Creates a new token with the specified name and initial balance.
    CreateToken {
        /// Random value use to create a unique token address.
        salt: u64,
        /// The name of the new token.
        token_name: String,
        /// The initial balance of the new token.
        initial_balance: Amount,
        /// The address of the account that minted new tokens.
        minter_address: C::Address,
    },

    /// Transfers a specified amount of tokens to the specified address.
    Transfer {
        /// The address to which the tokens will be transferred.
        to: C::Address,
        /// The amount of tokens to transfer.
        coins: Coins<C::Address>,
    },

    /// Burns a specified amount of tokens.
    Burn {
        /// The amount of tokens to burn.
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
        let (token_address, token) = Token::<C>::create(
            &token_name,
            &[(minter_address, initial_balance)],
            context.sender().as_ref(),
            salt,
            working_set,
        )?;

        if self.tokens.get(&token_address, working_set).is_some() {
            bail!("Token address already exists");
        }

        self.tokens.set(&token_address, token, working_set);
        Ok(CallResponse::default())
    }

    pub fn transfer(
        &self,
        to: C::Address,
        coins: Coins<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.transfer_from(context.sender(), &to, coins, working_set)
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
    pub fn transfer_from(
        &self,
        from: &C::Address,
        to: &C::Address,
        coins: Coins<C::Address>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token = self.tokens.get_or_err(&coins.token_address, working_set)?;
        token.transfer(from, to, coins.amount, working_set)
    }
}

pub(crate) fn prefix_from_address<C: sov_modules_api::Context>(
    token_address: &C::Address,
) -> sov_state::Prefix {
    sov_state::Prefix::new(token_address.as_ref().to_vec())
}
