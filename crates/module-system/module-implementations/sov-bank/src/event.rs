use sov_modules_api::macros::serialize;
use sov_modules_api::Spec;

use crate::utils::TokenHolder;
use crate::{Amount, Coins, TokenId};

/// Bank Event
#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
pub enum Event<S: Spec> {
    /// Event for Token Creation
    TokenCreated {
        /// The name of the new token.
        token_name: String,
        /// The new tokens that were minted.
        coins: Coins,
        /// The token holder that the new tokens are minted to.
        mint_to_address: TokenHolder<S>,
        /// The token holder that submitted the minting transaction.
        minter: TokenHolder<S>,
        /// The supply cap of the token.
        supply_cap: Amount,
        /// Admin list.
        admins: Vec<TokenHolder<S>>,
    },
    /// Event for Token Transfer
    TokenTransferred {
        /// The identity that is transferring the tokens.
        from: TokenHolder<S>,
        /// The token holder that the tokens were transferred to.
        to: TokenHolder<S>,
        /// The tokens transferred.
        coins: Coins,
    },
    /// Some tokens were burned
    TokenBurned {
        /// The owner that burnt the tokens.
        owner: TokenHolder<S>,
        /// The tokens that were burned.
        coins: Coins,
    },
    /// The supply of a token was frozen
    TokenFrozen {
        /// The token holder that froze the tokens
        freezer: TokenHolder<S>,
        /// The ID of the token that was transferred
        token_id: TokenId,
    },
    /// Event for Token Minting
    TokenMinted {
        /// The identity that authorized the tokens to be minted
        authorizer: TokenHolder<S>,
        /// The identity to mint the tokens to
        mint_to_identity: TokenHolder<S>,
        /// The coins minted
        coins: Coins,
    },
}
