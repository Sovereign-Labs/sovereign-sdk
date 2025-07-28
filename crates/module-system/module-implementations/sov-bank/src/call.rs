use anyhow::{bail, Context as _, Result};
use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    Context, EventEmitter, SafeString, SafeVec, Spec, StateAccessor, StateReader, TxState,
};
use sov_state::User;
use strum::{EnumDiscriminants, EnumIs, EnumIter, VariantArray};

use crate::event::Event;
use crate::token::unique_holders;
use crate::utils::{get_token_id_metered, Payable, TokenHolderRef};
use crate::{Amount, Bank, Coins, Token, TokenId};

/// The maximum number of addresses that can be authorized to mint or freeze a token.
pub const MAX_ADMINS: usize = 20;

/// This enumeration represents the available call messages for interacting with the sov-bank module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet, EnumDiscriminants, EnumIs)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
#[strum_discriminants(derive(VariantArray, EnumIs, EnumIter))]
pub enum CallMessage<S: Spec> {
    /// Creates a new token with the specified name and initial balance.
    CreateToken {
        /// The name of the new token.
        token_name: SafeString,
        /// The number of decimal places this token's amounts will have.
        token_decimals: Option<u8>,
        /// The initial balance of the new token.
        initial_balance: Amount,
        /// The address of the account that the new tokens are minted to.
        mint_to_address: S::Address,
        /// Admins list.
        admins: SafeVec<S::Address, MAX_ADMINS>,
        /// The supply cap of the new token, if any.
        supply_cap: Option<Amount>,
    },

    /// Transfers a specified amount of tokens to the specified address.
    #[sov_wallet(show_as = "Transfer to address {} {}.")]
    #[sov_wallet(template("transfer"))]
    Transfer {
        /// The address to which the tokens will be transferred.
        #[sov_wallet(template("transfer" = input("to")))]
        to: S::Address,
        /// The amount of tokens to transfer.
        coins: Coins,
    },

    /// Burns a specified amount of tokens.
    Burn {
        /// The amount of tokens to burn.
        coins: Coins,
    },

    /// Mints a specified amount of tokens.
    Mint {
        /// The amount of tokens to mint.
        coins: Coins,
        /// Address to mint tokens to
        mint_to_address: S::Address,
    },

    /// Freezes a token so that the supply is frozen
    Freeze {
        /// Address of the token to be frozen
        token_id: TokenId,
    },
    /// Updates the list of admins for a specified token.
    UpdateAdmin {
        /// The new admin address.
        /// If `None`, the current admin entry for the transaction sender will be removed.
        new_admin: Option<S::Address>,
        /// The ID of the token whose admin list is being updated.
        token_id: TokenId,
    },
}

impl<S: Spec> Bank<S> {
    /// Creates a token from a set of configuration parameters.
    /// Checks if a token already exists at that address. If so return an error.
    #[allow(clippy::too_many_arguments)]
    pub fn create_token(
        &mut self,
        token_name: String,
        token_decimals: Option<u8>,
        initial_balance: Amount,
        mint_to_address: impl Payable<S>,
        admins: Vec<impl Payable<S>>,
        supply_cap: Option<Amount>,
        minter: impl Payable<S>,
        state: &mut impl TxState<S>,
    ) -> Result<TokenId> {
        tracing::trace!(%minter, "Create token request");

        if let Some(decimals) = token_decimals {
            anyhow::ensure!(
                decimals <= Amount::MAX_DECIMALS,
                "Too many decimal places: {}, maximum allowed for a token: {}",
                decimals,
                Amount::MAX_DECIMALS
            );
        };

        if initial_balance > supply_cap.unwrap_or(Amount::MAX) {
            bail!(
                "Requested initial balance {} is greater than the supply cap {}",
                initial_balance,
                supply_cap.unwrap_or(Amount::MAX)
            );
        }

        let mint_to_address = mint_to_address.as_token_holder();
        let admins = admins
            .iter()
            .map(|minter| minter.as_token_holder())
            .collect::<Vec<_>>();

        let token_id = get_token_id_metered::<S>(&token_name, token_decimals, &minter, state)?;
        tracing::trace!(%token_name, originator = %minter, %token_id, "Calculated token id");
        let admins = unique_holders(&admins);
        let token = Token::<S> {
            name: token_name.to_owned(),
            total_supply: initial_balance,
            supply_cap: supply_cap.unwrap_or(Amount::MAX),
            admins: admins.clone(),
        };

        if self.tokens.get(&token_id, state)?.is_some() {
            bail!(
                "Token with id already exists {}, name={} minter={}",
                token_id,
                token_name,
                minter.as_token_holder()
            );
        }
        self.balances
            .set(&(mint_to_address, &token_id), &initial_balance, state)?;

        self.tokens.set(&token_id, &token, state)?;

        tracing::trace!(
            %token_id,
            %token_name,
            %minter,
            %initial_balance,
            %mint_to_address,
            ?admins,
            "Token created"
        );

        self.emit_event(
            state,
            Event::TokenCreated {
                token_name: token_name.clone(),
                coins: Coins {
                    amount: initial_balance,
                    token_id,
                },
                mint_to_address: mint_to_address.into(),
                minter: minter.as_token_holder().into(),
                supply_cap: supply_cap.unwrap_or(Amount::MAX),
                admins,
            },
        );
        Ok(token_id)
    }

    /// Transfers the set of `coins` to the address specified by `to`.
    pub fn transfer(
        &mut self,
        to: impl Payable<S>,
        coins: Coins,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        tracing::trace!("Transfer token request");

        let to = to.as_token_holder();
        let sender = context.sender();

        self.transfer_from(sender, to, coins.clone(), state)?;

        tracing::trace!(
            from = %sender,
            %to,
            %coins,
            "Token transfer successful"
        );

        self.emit_event(
            state,
            Event::TokenTransferred {
                from: sender.as_token_holder().into(),
                to: to.into(),
                coins,
            },
        );
        Ok(())
    }

    /// Burns (permanently destroys) the specified amount of tokens, removing them from circulation.
    /// This operation cannot be undone - burned tokens are permanently lost.
    ///
    /// # Errors
    ///
    /// If the specified token ID does not exist.
    ///
    /// If the `owner` has insufficient token balance to burn the requested amount.
    /// No tokens will be burned in this case.
    ///
    /// If the requested burn amount exceeds the token's total supply.
    /// No tokens will be burned in this case.
    pub fn burn(
        &mut self,
        coins: Coins,
        owner: impl Payable<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        tracing::trace!("Handling Burn call");

        let mut token = self
            .tokens
            .get_or_err(&coins.token_id, state)?
            .with_context(|| format!("Failed to get token_id={}", &coins.token_id))?;

        token.total_supply = token
            .total_supply
            .checked_sub(coins.amount)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Total supply underflow when burning, supply={} is less than burn amount={}",
                    token.total_supply,
                    coins.amount
                )
            })?;
        self.tokens.set(&coins.token_id, &token, state)?;

        let owner: TokenHolderRef<'_, S> = owner.as_token_holder();
        self.decrease_balance_checked(&coins.token_id, owner, coins.amount, state)?;
        tracing::trace!(
            id = %coins.token_id,
            name = token.name,
            burnt_amount = %coins.amount,
            %owner,
            updated_total_supply = %token.total_supply,
            "Successfully burnt tokens"
        );

        self.emit_event(
            state,
            Event::TokenBurned {
                owner: owner.into(),
                coins,
            },
        );

        Ok(())
    }

    /// Burns coins from an externally owned address ("EOA")
    pub(crate) fn burn_from_eoa(
        &mut self,
        coins: Coins,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let token_id = coins.token_id;
        self.burn(coins, context.sender(), state).with_context(|| {
            format!(
                "Failed to burn token_id={} owner={}",
                token_id,
                context.sender()
            )
        })?;
        Ok(())
    }

    /// Mints the `coins`to the address `mint_to_identity` using the externally owned account ("EOA") supplied by
    /// `context.sender()` as the authorizer.
    /// Returns an error if the token ID doesn't exist or `context.sender()` is not authorized to mint tokens.
    ///
    /// On success, it updates the `self.tokens` set to store the new balance.
    pub fn mint_from_eoa(
        &mut self,
        coins: Coins,
        mint_to_identity: impl Payable<S>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        self.mint(
            coins,
            mint_to_identity,
            TokenHolderRef::from(&context.sender()),
            state,
        )
    }

    /// Mints the `coins` to the  `mint_to_identity` if `authorizer` is an allowed minter.
    /// Returns an error if the token ID doesn't exist or `context.sender()` is not authorized to mint tokens.
    ///
    /// On success, it updates the `self.tokens` set to store the new minted address.
    pub fn mint(
        &mut self,
        coins: Coins,
        mint_to_identity: impl Payable<S>,
        authorizer: impl Payable<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        tracing::trace!(%authorizer, "Mint token request");

        let mint_to_identity = mint_to_identity.as_token_holder();
        let mut token = self
            .tokens
            .get_or_err(&coins.token_id, state)?
            .with_context(|| format!("Failed to get token_id={}", &coins.token_id))?;

        let authorizer = authorizer.as_token_holder();
        token
            .update_for_mint_if_allowed(authorizer, coins.amount)
            .with_context(|| format!("Failed to mint token_id={}", &coins.token_id))?;
        self.tokens.set(&coins.token_id, &token, state)?;

        let to_balance: Amount = self
            .balances
            .get(&(mint_to_identity, &coins.token_id), state)?
            .unwrap_or_default()
            .checked_add(coins.amount)
            .ok_or(anyhow::Error::msg(
                "Account balance overflow in the mint method of bank module",
            ))?;

        self.balances
            .set(&(mint_to_identity, &coins.token_id), &to_balance, state)?;

        tracing::trace!(
            %authorizer,
            token_id = %coins.token_id,
            amount = %coins.amount,
            minted_to = %mint_to_identity,
            "Successfully minted tokens"
        );

        self.emit_event(
            state,
            Event::TokenMinted {
                mint_to_identity: mint_to_identity.into(),
                authorizer: authorizer.into(),
                coins: coins.clone(),
            },
        );

        Ok(())
    }

    /// Insecure function to override the balance of an address for the gas token.
    /// This should only be used in VMs where the underlying transfers are black boxed (i.e. we trust the VM).
    pub fn override_gas_balance<Accessor: StateAccessor>(
        &mut self,
        balance: Amount,
        address: impl Payable<S>,
        state: &mut Accessor,
    ) -> Result<(), <Accessor as StateReader<User>>::Error> {
        self.balances.set(
            &(address.as_token_holder(), &crate::config_gas_token_id()),
            &balance,
            state,
        )?;

        Ok(())
    }

    /// Tries to freeze the token ID `token_id`.
    /// Returns an error if the token ID doesn't exist,
    /// otherwise calls the [`Token::freeze`] function, and update the token set upon success.
    pub(crate) fn freeze(
        &mut self,
        token_id: TokenId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let sender_ref = context.sender();
        let sender = sender_ref.as_token_holder();

        tracing::trace!(freezer = %sender, "Freeze token request");

        let mut token = self
            .tokens
            .get_or_err(&token_id, state)?
            .with_context(|| format!("Failed to get token_id={}", &token_id))?;

        token
            .freeze(sender)
            .with_context(|| format!("Failed to freeze token_id={}", &token_id))?;

        self.tokens.set(&token_id, &token, state)?;

        tracing::trace!(
            freezer = %sender,
            %token_id,
            "Successfully froze tokens"
        );

        self.emit_event(
            state,
            Event::TokenFrozen {
                freezer: sender.into(),
                token_id,
            },
        );

        Ok(())
    }

    /// Updates the admin list with the specified `new_admin`.
    /// If `new_admin` is `None`, the sender of the transaction is removed from the admin list.
    pub fn update_admin_address(
        &mut self,
        new_address: Option<S::Address>,
        token_id: TokenId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let mut token = self
            .tokens
            .get_or_err(&token_id, state)?
            .with_context(|| format!("Failed to get token_id={}", &token_id))?;

        token.update_admin(new_address, context.sender())?;
        self.tokens.set(&token_id, &token, state)?;
        Ok(())
    }
}

impl<S: Spec> Bank<S> {
    /// Transfers the set of `coins` from the address `from` to the address `to`.
    ///
    /// Returns an error if the token ID doesn't exist.
    pub fn transfer_from(
        &mut self,
        from: impl Payable<S>,
        to: impl Payable<S>,
        coins: Coins,
        state: &mut impl StateAccessor,
    ) -> Result<()> {
        let from = from.as_token_holder();
        let to = to.as_token_holder();

        self.do_transfer(from, to, &coins.token_id, coins.amount, state)
            .with_context(|| format!("Failed to transfer token_id={}", &coins.token_id))?;

        Ok(())
    }

    /// Transfer the amount `amount` of tokens from the address `from` to the address `to`.
    /// First checks that there is enough token of that type stored in `from`. If so, update
    /// the balances of the `from` and `to` accounts.
    fn do_transfer(
        &mut self,
        from: TokenHolderRef<'_, S>,
        to: TokenHolderRef<'_, S>,
        token_id: &TokenId,
        amount: Amount,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        if from == to {
            let balance = self
                .balances
                .get(&(from, token_id), state)?
                .unwrap_or(Amount::ZERO);

            if amount > balance {
                anyhow::bail!("Token transfer to self failed. Self: {from}, transfer amount: {amount}, balance: {balance}.")
            }

            tracing::trace!("Token transfer succeeded because it was transferring tokens to self.");
            return Ok(());
        }

        if amount == 0 {
            tracing::trace!("Token transfer succeeded because the transfer amount was zero.");
            return Ok(());
        }

        let from_balance = self.decrease_balance_checked(token_id, from, amount, state)?;

        let current_to_balance = self
            .balances
            .get(&(to, token_id), state)?
            .unwrap_or(Amount::ZERO);
        let to_balance = current_to_balance.checked_add(amount).with_context(|| {
            format!(
                "Account balance overflow for {to} when adding {amount} to current balance {current_to_balance}"
            )
        })?;

        self.balances.set(&(from, token_id), &from_balance, state)?;
        self.balances.set(&(to, token_id), &to_balance, state)?;
        Ok(())
    }
    // Check that amount can be deducted from address
    // Returns new balance after subtraction.
    fn decrease_balance_checked(
        &mut self,
        token_id: &TokenId,
        from: TokenHolderRef<'_, S>,
        amount: Amount,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<Amount> {
        let balance = self
            .balances
            .get(&(from, token_id), state)?
            .unwrap_or(Amount::ZERO);

        let new_balance = match balance.checked_sub(amount) {
            Some(from_balance) => from_balance,
            None => bail!(format!(
                "Insufficient balance from={from}, got={balance}, needed={amount}",
            )),
        };
        self.balances.set(&(from, token_id), &new_balance, state)?;
        Ok(new_balance)
    }

    /// Retrieve a token by the provided token id.
    pub fn get_token<Accessor: StateAccessor>(
        &self,
        token_id: &TokenId,
        state: &mut Accessor,
    ) -> Result<Option<Token<S>>, <Accessor as StateReader<User>>::Error> {
        self.tokens.get(token_id, state)
    }

    /// Helper function to return the balance of the token stored at `token_id`
    /// for the user having the address `user_address` from the underlying storage. If the token ID doesn't exist, or
    /// if the user doesn't have tokens of that type, return `None`. Otherwise, wrap the resulting balance in `Some`.
    pub fn get_balance_of<Accessor: StateAccessor>(
        &self,
        user_address: impl Payable<S>,
        token_id: TokenId,
        state: &mut Accessor,
    ) -> Result<Option<Amount>, <Accessor as StateReader<User>>::Error> {
        let user_address = user_address.as_token_holder();
        self.balances.get(&(user_address, &token_id), state)
    }

    /// Get the name of a token by ID
    pub fn get_token_name<Accessor: StateReader<User>>(
        &self,
        token_id: &TokenId,
        state: &mut Accessor,
    ) -> Result<Option<String>, Accessor::Error> {
        let token = self.tokens.get(token_id, state)?;
        Ok(token.map(|token| token.name))
    }

    /// Returns the total supply of the token with the given `token_id`.
    pub fn get_total_supply_of<Accessor: StateAccessor>(
        &self,
        token_id: &TokenId,
        state: &mut Accessor,
    ) -> Result<Option<Amount>, <Accessor as StateReader<User>>::Error> {
        Ok(self
            .tokens
            .get(token_id, state)?
            .map(|token| token.total_supply))
    }
}
