//! Implements call message generation for the [`sov_bank::Bank`] module.
use std::marker::PhantomData;
use std::sync::Arc;

use derivative::Derivative;
use serde::{Deserialize, Serialize};
use sov_bank::{Amount, CallMessage, CallMessageDiscriminants, Coins, TokenId};
use sov_modules_api::prelude::arbitrary;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::Spec;
use strum::VariantArray;
use tracing::warn;

mod burn;
mod freeze;
mod mint;
mod query;
pub use query::http::HttpBankClient;
mod account;
mod create_token;

/// The traits to be implemented to interface with the harness
pub mod harness_interface;

mod transfer;
pub use account::BankAccount;

use crate::interface::{
    CallMessageGenerator, Distribution, GeneratedMessage, GeneratorState, MessageValidity, Percent,
};
use crate::ChangelogEntry;

/// The call message discriminants used by the `Bank` module
pub const MESSAGES: &[sov_bank::CallMessageDiscriminants] =
    sov_bank::CallMessageDiscriminants::VARIANTS;

/// A generator for bank call messages.
#[derive(Debug, Clone)]
pub struct BankMessageGenerator<S> {
    message_distribution: Distribution<CallMessageDiscriminants>,
    // The fraction of valid messages that should create a new address. This may be
    // any valid percent from 0 to 100 (inclusive).
    pub(crate) address_creation_rate: Percent,
    phantom: PhantomData<S>,
}

impl<S: Spec> BankMessageGenerator<S> {
    /// Instantiate a new [`BankMessageGenerator`]
    pub fn new(
        message_distribution: Distribution<CallMessageDiscriminants>,
        address_creation_rate: Percent,
    ) -> Self {
        Self {
            message_distribution,
            address_creation_rate,
            phantom: PhantomData,
        }
    }

    /// Performs callmessage generation, falling back to variants that are more likely to succeed with limited state
    fn do_generation_with_fallback(
        &self,
        message_type: CallMessageDiscriminants,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<S, AccountView = BankAccount<S>, Tag: From<BankTag>>,
        validity: MessageValidity,
    ) -> InternalMessageGenResult<GeneratedMessage<S, CallMessage<S>, BankChangeLogEntry<S>>> {
        match (message_type, validity) {
            (CallMessageDiscriminants::Transfer, MessageValidity::Valid) => {
                match self
                    .generate_valid_transfer(u, generator_state)
                    .try_to_arbitrary()
                {
                    Ok(transfer_result) => Ok(transfer_result?),
                    Err(e) => {
                        warn!(
                            "Failed to generate transfer: {:?}. Generating mint instead",
                            e
                        );

                        self.do_generation_with_fallback(
                            CallMessageDiscriminants::Mint,
                            u,
                            generator_state,
                            validity,
                        )
                    }
                }
            }
            (CallMessageDiscriminants::Transfer, MessageValidity::Invalid) => {
                Ok(self.generate_invalid_transfer(u, generator_state)?)
            }
            (CallMessageDiscriminants::CreateToken, MessageValidity::Valid) => {
                Ok(self.generate_valid_create_token(u, generator_state)?)
            }
            (CallMessageDiscriminants::CreateToken, MessageValidity::Invalid) => {
                match self
                    .generate_invalid_create_token(u, generator_state)
                    .try_to_arbitrary()
                {
                    Ok(create_result) => Ok(create_result?),
                    Err(e) => {
                        warn!("Failed to generate create token: {:?}", e);

                        // Fall back to generating an *invalid* transfer, which is always possible
                        self.do_generation_with_fallback(
                            CallMessageDiscriminants::Transfer,
                            u,
                            generator_state,
                            validity,
                        )
                    }
                }
            }
            (CallMessageDiscriminants::Burn, MessageValidity::Valid) => {
                match self
                    .generate_valid_burn(u, generator_state)
                    .try_to_arbitrary()
                {
                    Ok(transfer_result) => Ok(transfer_result?),
                    Err(e) => {
                        warn!("Failed to generate burn: {:?}", e);
                        self.do_generation_with_fallback(
                            CallMessageDiscriminants::CreateToken,
                            u,
                            generator_state,
                            validity,
                        )
                    }
                }
            }
            (CallMessageDiscriminants::Burn, MessageValidity::Invalid) => {
                Ok(self.generate_invalid_burn(u, generator_state)?)
            }
            (CallMessageDiscriminants::Mint, MessageValidity::Valid) => {
                match self
                    .generate_valid_mint(u, generator_state)
                    .try_to_arbitrary()
                {
                    Ok(transfer_result) => Ok(transfer_result?),
                    Err(e) => {
                        warn!("Failed to generate mint: {:?}", e);
                        self.do_generation_with_fallback(
                            CallMessageDiscriminants::CreateToken,
                            u,
                            generator_state,
                            validity,
                        )
                    }
                }
            }
            (CallMessageDiscriminants::Mint, MessageValidity::Invalid) => {
                Ok(self.generate_invalid_mint(u, generator_state)?)
            }
            (CallMessageDiscriminants::Freeze, MessageValidity::Valid) => {
                match self
                    .generate_valid_freeze(u, generator_state)
                    .try_to_arbitrary()
                {
                    Ok(transfer_result) => Ok(transfer_result?),
                    Err(e) => {
                        warn!("Failed to generate freeze: {:?}", e);
                        self.do_generation_with_fallback(
                            CallMessageDiscriminants::CreateToken,
                            u,
                            generator_state,
                            validity,
                        )
                    }
                }
            }
            (CallMessageDiscriminants::Freeze, MessageValidity::Invalid) => {
                Ok(self.generate_invalid_freeze(u, generator_state)?)
            }
            (CallMessageDiscriminants::UpdateAdmin, MessageValidity::Valid) => {
                todo!("https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/3235")
            }
            (CallMessageDiscriminants::UpdateAdmin, MessageValidity::Invalid) => {
                todo!("https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/3235")
            }
        }
    }
}

/// A complete description of any possible state change created by the bank message generator.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BankChangeLogEntry<S: Spec> {
    /// The balance of an address changed
    BalanceChanged {
        /// The address for which the balance was changed
        address: S::Address,
        /// The balance after the change
        coins: Coins,
    },

    /// The supply of a token changed
    SupplyChanged {
        /// The token id for which the supply was changed
        token_id: TokenId,
        /// The total supply after the change
        total_supply: Amount,
    },

    /// The token was frozen
    TokenFrozen {
        /// The token id for which the freeze event was triggered
        token_id: TokenId,
    },
}

/// Helper struct that can be used to discriminate between different [`BankChangeLogEntry`]s.
#[derive(Debug, PartialEq, Eq, Derivative)]
#[derivative(Hash(bound = ""))]
pub enum BankChangeLogDiscriminant<S: Spec> {
    /// A balance was changed.
    BalanceChanged {
        /// The address for which the balance was changed.
        address: S::Address,
    },

    /// A token supply was changed.
    SupplyChanged {
        /// The token id for which the supply was changed.
        token_id: TokenId,
    },

    /// A token was frozen.
    TokenFrozen {
        /// The token id for which the freeze event was triggered.
        token_id: TokenId,
    },
}

#[async_trait]
impl<S: Spec> ChangelogEntry for BankChangeLogEntry<S> {
    type ClientConfig = HttpBankClient<S>;

    type Discriminant = BankChangeLogDiscriminant<S>;

    async fn assert_state(
        &self,
        rollup_state_accessor: Arc<Self::ClientConfig>,
    ) -> Result<(), anyhow::Error> {
        match self {
            BankChangeLogEntry::BalanceChanged { address, coins } => {
                let Coins { token_id, amount } = coins;
                let found_balance = &rollup_state_accessor.get_balance(address, *token_id).await;
                assert_eq!(
                    found_balance, amount,
                    "Unexpected balance of {} at address {}",
                    token_id, &address
                );
            }
            BankChangeLogEntry::SupplyChanged {
                token_id,
                total_supply,
            } => {
                let found_supply = &rollup_state_accessor.get_total_supply(token_id).await;
                assert_eq!(
                    found_supply, total_supply,
                    "Unexpected total supply of {token_id}",
                );
            }
            BankChangeLogEntry::TokenFrozen { token_id } => {
                assert!(
                    rollup_state_accessor.is_frozen(token_id).await,
                    "Token with id {token_id} should be frozen"
                );
            }
        }

        Ok(())
    }

    fn as_discriminant(&self) -> Self::Discriminant {
        match self {
            BankChangeLogEntry::BalanceChanged { address, .. } => {
                BankChangeLogDiscriminant::BalanceChanged {
                    address: address.clone(),
                }
            }
            BankChangeLogEntry::SupplyChanged { token_id, .. } => {
                BankChangeLogDiscriminant::SupplyChanged {
                    token_id: *token_id,
                }
            }
            BankChangeLogEntry::TokenFrozen { token_id } => {
                BankChangeLogDiscriminant::TokenFrozen {
                    token_id: *token_id,
                }
            }
        }
    }
}

#[async_trait]
impl<S: Spec> CallMessageGenerator<S> for BankMessageGenerator<S> {
    type Module = sov_bank::Bank<S>;

    type AccountView = BankAccount<S>;

    type Tag = BankTag;

    type ChangelogEntry = BankChangeLogEntry<S>;

    fn generate_setup_messages(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
    ) -> arbitrary::Result<Vec<GeneratedMessage<S, sov_bank::CallMessage<S>, Self::ChangelogEntry>>>
    {
        let GeneratedMessage {
            message,
            sender,
            outcome,
        } = self
            .generate_valid_create_token_with_creation_rate(
                Percent::one_hundred(),
                u,
                generator_state,
            )
            .expect("Valid token creation can't fail");

        Ok(vec![GeneratedMessage {
            message,
            sender,
            outcome,
        }])
    }

    fn generate_call_message(
        &self,
        u: &mut arbitrary::Unstructured<'_>,
        generator_state: &mut impl GeneratorState<
            S,
            AccountView = Self::AccountView,
            Tag: From<Self::Tag>,
        >,
        validity: MessageValidity,
    ) -> arbitrary::Result<GeneratedMessage<S, sov_bank::CallMessage<S>, Self::ChangelogEntry>>
    {
        let message = *self.message_distribution.select_value(u)?;
        self.do_generation_with_fallback(message, u, generator_state, validity)
            .try_to_arbitrary()
            .expect("Could not generate bank callmessage")
    }
}

/// A tag used for indexing by the bank message generator
#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum BankTag {
    /// Accounts which have a balance of some token. These can be used to generate transfers.
    HasBalance,
    /// Accounts which are allowed to mint some token.
    CanMint,
    /// Accounts which are allowed to mint some particular token.
    CanMintById(TokenId),
    /// Accounts which have created a token.
    HasCreatedToken,
}

/// An error generated during message generation
#[derive(thiserror::Error, Debug)]
pub(crate) enum InternalMessageGenError {
    #[error(transparent)]
    Arbitrary(#[from] arbitrary::Error),
    /// A transfer could not be generated because no account with sufficient balance was found.
    // Note: If no account with balance can be found, we can simply try to generate
    // a create or mint token message.
    #[error("Could not find an account with available token balance")]
    NoAccountWithBalance,
    /// A mint could not be generated because no account without appropriate permissions could be found
    #[error("Could not find an account that is authorized to mint")]
    NoMintingAccounts,
    /// A mint could not be generated because no account could receive the token
    #[error("Could not find an account can receive {0}")]
    NoAccountsCanReceive(Coins),
    /// A create token can only fail if the account has already created a token with the same name,
    /// so generating an invalid `create_token` can fail in this case
    #[error("Could not find an account that has created a token")]
    NoAccountsHaveCreatedTokensYet,
}

type InternalMessageGenResult<T, E = InternalMessageGenError> = Result<T, E>;

/// Try to convert a result type *containing* `Arbitrary::Error` to an `arbitrary::Result`, failing
/// only if the type is `Err` *and* the contained error is not an arbitrary error.
trait TryToArbitrary<T>: Sized {
    fn try_to_arbitrary(self) -> Result<arbitrary::Result<T>, Self>;
}

impl<T> TryToArbitrary<T> for InternalMessageGenResult<T> {
    fn try_to_arbitrary(self) -> Result<arbitrary::Result<T>, Self> {
        match self {
            Ok(ok) => Ok(Ok(ok)),
            Err(InternalMessageGenError::Arbitrary(e)) => Ok(Err(e)),
            _ => Err(self),
        }
    }
}
