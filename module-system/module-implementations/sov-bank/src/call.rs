use anyhow::{bail, Result};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::{Amount, Bank, Coins, Token};

/// This enumeration represents the available call messages for interacting with the sov-bank module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
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
        /// The address of the account that the new tokens are minted to.
        minter_address: C::Address,
        /// Authorized minter list.
        authorized_minters: Vec<C::Address>,
    },

    /// Transfers a specified amount of tokens to the specified address.
    Transfer {
        /// The address to which the tokens will be transferred.
        to: C::Address,
        /// The amount of tokens to transfer.
        coins: Coins<C>,
    },

    /// Burns a specified amount of tokens.
    Burn {
        /// The amount of tokens to burn.
        coins: Coins<C>,
    },

    /// Mints a specified amount of tokens.
    Mint {
        /// The amount of tokens to mint.
        coins: Coins<C>,
        /// Address to mint tokens to
        minter_address: C::Address,
    },

    /// Freeze a token so that the supply is frozen
    Freeze {
        /// Address of the token to be frozen
        token_address: C::Address,
    },
}

impl<C: sov_modules_api::Context> Bank<C> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_token(
        &self,
        token_name: String,
        salt: u64,
        initial_balance: Amount,
        minter_address: C::Address,
        authorized_minters: Vec<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let (token_address, token) = Token::<C>::create(
            &token_name,
            &[(minter_address, initial_balance)],
            authorized_minters,
            context.sender().as_ref(),
            salt,
            self.tokens.prefix(),
            working_set,
        )?;

        if self.tokens.get(&token_address, working_set).is_some() {
            bail!(
                "Token {} at {} address already exists",
                token_name,
                token_address
            );
        }

        self.tokens.set(&token_address, &token, working_set);
        Ok(CallResponse::default())
    }

    pub fn transfer(
        &self,
        to: C::Address,
        coins: Coins<C>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.transfer_from(context.sender(), &to, coins, working_set)
    }

    pub(crate) fn burn(
        &self,
        coins: Coins<C>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let mut token = self.tokens.get_or_err(&coins.token_address, working_set)?;
        token.burn(context.sender(), coins.amount, working_set)?;
        token.total_supply -= coins.amount;
        self.tokens.set(&coins.token_address, &token, working_set);

        Ok(CallResponse::default())
    }

    pub(crate) fn mint(
        &self,
        coins: Coins<C>,
        minter_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let mut token = self.tokens.get_or_err(&coins.token_address, working_set)?;
        token.mint(context.sender(), &minter_address, coins.amount, working_set)?;
        self.tokens.set(&coins.token_address, &token, working_set);

        Ok(CallResponse::default())
    }

    pub(crate) fn freeze(
        &self,
        token_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let mut token = self.tokens.get_or_err(&token_address, working_set)?;
        token.freeze(context.sender())?;
        self.tokens.set(&token_address, &token, working_set);

        Ok(CallResponse::default())
    }
}

impl<C: sov_modules_api::Context> Bank<C> {
    pub fn transfer_from(
        &self,
        from: &C::Address,
        to: &C::Address,
        coins: Coins<C>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let token = self.tokens.get_or_err(&coins.token_address, working_set)?;
        token.transfer(from, to, coins.amount, working_set)?;
        Ok(CallResponse::default())
    }
}

pub(crate) fn prefix_from_address_with_parent<C: sov_modules_api::Context>(
    parent_prefix: &sov_state::Prefix,
    token_address: &C::Address,
) -> sov_state::Prefix {
    let mut prefix = parent_prefix.as_aligned_vec().clone().into_inner();
    prefix.extend_from_slice(format!("{}", token_address).as_bytes());
    sov_state::Prefix::new(prefix)
}
