use anyhow::{bail, Context, Result};
#[cfg(feature = "native")]
use sov_modules_api::macros::CliWalletArg;
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::{Amount, Bank, Coins, Token};

/// This enumeration represents the available call messages for interacting with the sov-bank module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(CliWalletArg),
    derive(schemars::JsonSchema),
    schemars(bound = "C::Address: ::schemars::JsonSchema", rename = "CallMessage")
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

    /// Freezes a token so that the supply is frozen
    Freeze {
        /// Address of the token to be frozen
        token_address: C::Address,
    },
}

impl<C: sov_modules_api::Context> Bank<C> {
    /// Creates a token from a set of configuration parameters.
    /// Checks if a token already exists at that address. If so return an error.
    #[allow(clippy::too_many_arguments)]
    pub fn create_token(
        &self,
        token_name: String,
        salt: u64,
        initial_balance: Amount,
        minter_address: C::Address,
        authorized_minters: Vec<C::Address>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<C::Address> {
        let (token_address, token) = Token::<C>::create(
            &token_name,
            &[(minter_address, initial_balance)],
            &authorized_minters,
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
        Ok(token_address)
    }

    /// Transfers the set of `coins` to the address specified by `to`.
    pub fn transfer(
        &self,
        to: C::Address,
        coins: Coins<C>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.transfer_from(context.sender(), &to, coins, working_set)
    }

    /// Burns the set of `coins`.
    ///
    /// If there is no token at the address specified in the
    /// [`Coins`] structure, return an error; on success it updates the total
    /// supply of tokens.
    pub fn burn(
        &self,
        coins: Coins<C>,
        owner: &C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let context_logger = || format!("Failed to burn coins({}) from owner {}", coins, owner,);
        let mut token = self
            .tokens
            .get_or_err(&coins.token_address, working_set)
            .with_context(context_logger)?;
        token
            .burn(owner, coins.amount, working_set)
            .with_context(context_logger)?;
        token.total_supply -= coins.amount;
        self.tokens.set(&coins.token_address, &token, working_set);

        Ok(())
    }

    /// Burns coins from an externally owned address ("EOA")
    pub(crate) fn burn_from_eoa(
        &self,
        coins: Coins<C>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.burn(coins, context.sender(), working_set)?;
        Ok(CallResponse::default())
    }

    /// Mints the `coins`to the address `mint_to_address` using the externally owned account ("EOA") supplied by
    /// `context.sender()` as the authorizer.
    /// Returns an error if the token address doesn't exist or `context.sender()` is not authorized to mint tokens.
    ///
    /// On success, it updates the `self.tokens` set to store the new balance.
    pub fn mint_from_eoa(
        &self,
        coins: &Coins<C>,
        mint_to_address: &C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.mint(coins, mint_to_address, context.sender(), working_set)
    }

    /// Mints the `coins` to the address `mint_to_address` if `authorizer` is an allowed minter.
    /// Returns an error if the token address doesn't exist or `context.sender()` is not authorized to mint tokens.
    ///
    /// On success, it updates the `self.tokens` set to store the new minted address.
    pub fn mint(
        &self,
        coins: &Coins<C>,
        mint_to_address: &C::Address,
        authorizer: &C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let context_logger = || {
            format!(
                "Failed mint coins({}) to {} by authorizer {}",
                coins, mint_to_address, authorizer
            )
        };
        let mut token = self
            .tokens
            .get_or_err(&coins.token_address, working_set)
            .with_context(context_logger)?;
        token
            .mint(authorizer, mint_to_address, coins.amount, working_set)
            .with_context(context_logger)?;
        self.tokens.set(&coins.token_address, &token, working_set);

        Ok(())
    }

    /// Tries to freeze the token address `token_address`.
    /// Returns an error if the token address doesn't exist,
    /// otherwise calls the [`Token::freeze`] function, and update the token set upon success.
    pub(crate) fn freeze(
        &self,
        token_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let context_logger = || {
            format!(
                "Failed freeze token_address={} by sender {}",
                token_address,
                context.sender()
            )
        };
        let mut token = self
            .tokens
            .get_or_err(&token_address, working_set)
            .with_context(context_logger)?;
        token
            .freeze(context.sender())
            .with_context(context_logger)?;
        self.tokens.set(&token_address, &token, working_set);

        Ok(CallResponse::default())
    }
}

impl<C: sov_modules_api::Context> Bank<C> {
    /// Transfers the set of `coins` from the address `from` to the address `to`.
    ///
    /// Returns an error if the token address doesn't exist.
    pub fn transfer_from(
        &self,
        from: &C::Address,
        to: &C::Address,
        coins: Coins<C>,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let context_logger = || {
            format!(
                "Failed transfer from={} to={} of coins({})",
                from, to, coins
            )
        };
        let token = self
            .tokens
            .get_or_err(&coins.token_address, working_set)
            .with_context(context_logger)?;
        token
            .transfer(from, to, coins.amount, working_set)
            .with_context(context_logger)?;
        Ok(CallResponse::default())
    }

    /// Helper function used by the rpc method [`balance_of`](Bank::balance_of) to return the balance of the token stored at `token_address`
    /// for the user having the address `user_address` from the underlying storage. If the token address doesn't exist, or
    /// if the user doesn't have tokens of that type, return `None`. Otherwise, wrap the resulting balance in `Some`.
    pub fn get_balance_of(
        &self,
        user_address: C::Address,
        token_address: C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<u64> {
        self.tokens
            .get(&token_address, working_set)
            .and_then(|token| token.balances.get(&user_address, working_set))
    }
}

/// Creates a new prefix from an already existing prefix `parent_prefix` and a `token_address`
/// by extending the parent prefix.
pub(crate) fn prefix_from_address_with_parent<C: sov_modules_api::Context>(
    parent_prefix: &sov_state::Prefix,
    token_address: &C::Address,
) -> sov_state::Prefix {
    let mut prefix = parent_prefix.as_aligned_vec().clone().into_inner();
    prefix.extend_from_slice(format!("{}", token_address).as_bytes());
    sov_state::Prefix::new(prefix)
}
