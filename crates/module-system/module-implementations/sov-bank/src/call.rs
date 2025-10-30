use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    Context, CoreModuleError, EventEmitter, ModuleInfo, RuntimeDiscriminant, SafeString, SafeVec,
    Spec, StateAccessor, StateReader, TxState,
};
use sov_rollup_interface::common::SizedSafeString;
use sov_state::{EventContainer, User};
use strum::{EnumDiscriminants, EnumIs, EnumIter, VariantArray};

use crate::error::{
    ArithmeticError, BurnTokenError, CommonError, CreateTokenError, FreezeTokenError,
    MintTokenError, TransferTokenError, UpdateAdminError,
};
use crate::event::Event;
use crate::token::unique_holders;
use crate::utils::{get_token_id_metered, Payable, TokenHolderRef};
use crate::{Amount, Bank, Coins, Token, TokenId};

/// The maximum length of the memo field for a transfer, in bytes.
pub const MAX_MEMO_LENGTH: usize = 512;

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
    /// Transfers a specified amount of tokens to the specified address.
    #[sov_wallet(show_as = "Transfer to address {} {} with memo `{}`.")]
    TransferWithMemo {
        /// The address to which the tokens will be transferred.
        to: S::Address,
        /// The amount of tokens to transfer.
        coins: Coins,
        /// The message included with the transfer
        memo: SizedSafeString<MAX_MEMO_LENGTH>,
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
    ) -> Result<TokenId, CreateTokenError> {
        tracing::trace!(%minter, "Create token request");

        if let Some(decimals) = token_decimals {
            if decimals > Amount::MAX_DECIMALS {
                return Err(CreateTokenError::TooManyDecimals {
                    provided: decimals,
                    max_allowed: Amount::MAX_DECIMALS,
                });
            }
        };

        let supply_cap = supply_cap.unwrap_or(Amount::MAX);

        if initial_balance > supply_cap {
            return Err(CreateTokenError::InitialBalanceExceedsSupplyCap {
                initial_balance,
                supply_cap,
            });
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
            supply_cap,
            admins: admins.clone(),
        };

        let token_exists = self
            .tokens
            .get(&token_id, state)
            .map_err(CoreModuleError::state_read)?
            .is_some();

        if token_exists {
            return Err(CreateTokenError::TokenAlreadyExists {
                token_id: token_id.to_string(),
                name: token_name,
                minter: minter.as_token_holder().to_string(),
            })?;
        }

        self.balances
            .set(&(mint_to_address, &token_id), &initial_balance, state)
            .map_err(CoreModuleError::state_write)?;

        self.tokens
            .set(&token_id, &token, state)
            .map_err(CoreModuleError::state_write)?;

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
                supply_cap,
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
    ) -> Result<(), TransferTokenError> {
        self.transfer_with_memo(to, coins, None, context, state)
    }

    /// Transfers the set of `coins` to the address specified by `to` with an optional memo.
    pub fn transfer_with_memo(
        &mut self,
        to: impl Payable<S>,
        coins: Coins,
        memo: Option<String>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), TransferTokenError> {
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
                memo,
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
    ) -> Result<(), BurnTokenError> {
        tracing::trace!("Handling Burn call");

        let mut token = self
            .tokens
            .get(&coins.token_id, state)
            .map_err(CoreModuleError::state_read)?
            .ok_or_else(|| CommonError::TokenNotFound {
                token_id: coins.token_id,
            })?;

        token.total_supply = token
            .total_supply
            .checked_sub(coins.amount)
            .ok_or_else(|| {
                CommonError::Arithmetic(ArithmeticError::Underflow {
                    base: token.total_supply,
                    subtrahend: coins.amount,
                    message: "Total supply underflow when burning".to_owned(),
                })
            })?;
        self.tokens
            .set(&coins.token_id, &token, state)
            .map_err(CoreModuleError::state_write)?;

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
    ) -> Result<(), BurnTokenError> {
        self.burn(coins, context.sender(), state)
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
    ) -> Result<(), MintTokenError> {
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
    ) -> Result<(), MintTokenError> {
        tracing::trace!(%authorizer, "Mint token request");

        let mint_to_identity = mint_to_identity.as_token_holder();
        let mut token = self
            .tokens
            .get(&coins.token_id, state)
            .map_err(CoreModuleError::state_read)?
            .ok_or_else(|| CommonError::TokenNotFound {
                token_id: coins.token_id,
            })?;

        let authorizer = authorizer.as_token_holder();
        token.update_for_mint_if_allowed(authorizer, coins.amount)?;
        self.tokens
            .set(&coins.token_id, &token, state)
            .map_err(CoreModuleError::state_write)?;

        let current_to_balance = self
            .balances
            .get(&(mint_to_identity, &coins.token_id), state)
            .map_err(CoreModuleError::state_read)?
            .unwrap_or_default();
        let to_balance: Amount = current_to_balance
            .checked_add(coins.amount)
            .ok_or_else(|| {
                CommonError::Arithmetic(ArithmeticError::Overflow {
                    base: current_to_balance,
                    addend: coins.amount,
                    message: format!("Mint account balance overflow {mint_to_identity}"),
                })
            })?;

        self.balances
            .set(&(mint_to_identity, &coins.token_id), &to_balance, state)
            .map_err(CoreModuleError::state_write)?;

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
    ) -> Result<(), FreezeTokenError> {
        let sender_ref = context.sender();
        let sender = sender_ref.as_token_holder();

        tracing::trace!(freezer = %sender, "Freeze token request");

        let mut token = self
            .tokens
            .get(&token_id, state)
            .map_err(CoreModuleError::state_read)?
            .ok_or_else(|| CommonError::TokenNotFound { token_id })?;

        token.freeze(sender)?;

        self.tokens
            .set(&token_id, &token, state)
            .map_err(CoreModuleError::state_write)?;

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
    ) -> Result<(), UpdateAdminError> {
        let mut token = self
            .tokens
            .get(&token_id, state)
            .map_err(CoreModuleError::state_read)?
            .ok_or_else(|| CommonError::TokenNotFound { token_id })?;

        token.update_admin(new_address, context.sender())?;
        self.tokens
            .set(&token_id, &token, state)
            .map_err(CoreModuleError::state_write)?;
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
    ) -> Result<(), TransferTokenError> {
        let from = from.as_token_holder();
        let to = to.as_token_holder();

        self.do_transfer(from, to, &coins.token_id, coins.amount, state)
    }

    /// Transfers the set of `coins` from the address `from` to the address `to` with an optional memo.
    ///
    /// Returns an error if the token ID doesn't exist.
    pub fn transfer_from_with_memo(
        &mut self,
        from: impl Payable<S>,
        to: impl Payable<S>,
        coins: Coins,
        memo: Option<String>,
        state: &mut (impl StateAccessor + EventContainer),
    ) -> Result<(), TransferTokenError> {
        let from = from.as_token_holder();
        let to = to.as_token_holder();

        self.do_transfer(from, to, &coins.token_id, coins.amount, state)?;

        self.emit_event(
            state,
            Event::TokenTransferred {
                from: from.into(),
                to: to.into(),
                coins,
                memo,
            },
        );

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
    ) -> Result<(), TransferTokenError> {
        if from == to {
            let balance = self
                .balances
                .get(&(from, token_id), state)
                .map_err(CoreModuleError::state_read)?
                .unwrap_or(Amount::ZERO);

            if amount > balance {
                return Err(TransferTokenError::InsufficientBalance {
                    amount,
                    balance,
                    from: from.to_string(),
                    to: to.to_string(),
                    token_id: token_id.to_string(),
                });
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
            .get(&(to, token_id), state)
            .map_err(CoreModuleError::state_read)?
            .unwrap_or(Amount::ZERO);

        let to_balance = current_to_balance.checked_add(amount).ok_or_else(|| {
            CommonError::Arithmetic(ArithmeticError::Overflow {
                base: current_to_balance,
                addend: amount,
                message: format!("Balance overflow for account {to}"),
            })
        })?;

        self.balances
            .set(&(from, token_id), &from_balance, state)
            .map_err(CoreModuleError::state_write)?;
        self.balances
            .set(&(to, token_id), &to_balance, state)
            .map_err(CoreModuleError::state_write)?;
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
    ) -> Result<Amount, CommonError> {
        let balance = self
            .balances
            .get(&(from, token_id), state)
            .map_err(CoreModuleError::state_read)?
            .unwrap_or(Amount::ZERO);

        let new_balance = balance.checked_sub(amount).ok_or_else(|| {
            CommonError::Arithmetic(ArithmeticError::Underflow {
                base: balance,
                subtrahend: amount,
                message: format!("Insufficient balance for account {from}"),
            })
        })?;

        self.balances
            .set(&(from, token_id), &new_balance, state)
            .map_err(CoreModuleError::state_write)?;
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

impl<S: Spec> RuntimeDiscriminant for CallMessage<S> {
    fn runtime_discriminant() -> u8 {
        crate::Bank::<S>::default().discriminant()
    }
}
