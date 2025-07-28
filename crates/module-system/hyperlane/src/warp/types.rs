use std::cmp::Ordering;

use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_bank::TokenId;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Amount, HexHash, Spec, VersionReader, VisibleSlotNumber};

use crate::Ism;

/// Represents the source of the token in Hyperlane
#[derive(
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Clone,
    UniversalWallet,
    Eq,
    JsonSchema,
    Hash,
    strum::EnumDiscriminants,
)]
#[strum_discriminants(derive(BorshSerialize, BorshDeserialize, strum::AsRefStr))]
#[strum_discriminants(name(TokenKindId))]
pub enum TokenKind {
    /// The token is natively issued on some remote chain, so the local representation is a synthetic token.
    // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/9334f3345724886953bdc980b2dff717b80bb87c/solidity/contracts/token/HypERC20.sol#L17
    Synthetic {
        /// The ID of the remote token.
        remote_token_id: HexHash,
        /// The number of decimal places of the remote token.
        remote_decimals: u8,
        /// The number of decimal places for the local (synthetic) token.
        ///
        /// Should be set if remote token should be scaled locally, defaults to remote decimals.
        // NOTE: this implementation follows the sealevel implementation of scaling rather
        // than solidity's one. Solidity uses `decimals` which isn't involved in computations
        // and `scale`, but only allowing scaling down by arbitrary unsigned integer.
        // sealevel <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/main/rust/sealevel/libraries/hyperlane-sealevel-token/src/accounts.rs#L77>
        // evm <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/9334f3345724886953bdc980b2dff717b80bb87c/solidity/contracts/token/libs/FungibleTokenRouter.sol>
        local_decimals: Option<u8>,
    },
    /// The token is natively issued on the local chain.
    Collateral {
        /// The ID of the token on the local chain.
        token: TokenId,
    },
    /// The token is the native token of the local chain.
    Native,
}

/// Represents the source of the token in Hyperlane
#[derive(
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Clone,
    UniversalWallet,
    Eq,
    JsonSchema,
    Hash,
)]
pub enum StoredTokenKind {
    /// The token is natively issued on some remote chain, so the local representation is a synthetic token.
    // https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/9334f3345724886953bdc980b2dff717b80bb87c/solidity/contracts/token/HypERC20.sol#L17
    Synthetic {
        /// The ID of the remote token.
        remote_token_id: HexHash,
        /// The number of decimal places for the local (synthetic) token.
        local_decimals: u8,
        /// The number of decimal places of the remote token.
        remote_decimals: u8,
        /// The token ID of the token on the *local* chain.
        local_token_id: TokenId,
    },
    /// The token is natively issued on the local chain.
    Collateral {
        /// The ID of the token on the local chain.
        token: TokenId,
    },
    /// The token is the native token of the local chain.
    Native,
}

#[derive(
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Clone,
    UniversalWallet,
    Eq,
    JsonSchema,
    Hash,
)]
/// The address of a remote router.
pub struct RemoteRouterAddress(pub HexHash);

impl std::fmt::Display for RemoteRouterAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for RemoteRouterAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(HexHash::from_str(s)?))
    }
}

type U256 = ruint::Uint<256, 4>;

/// Converts an amount from one decimal representation to another.
fn convert_decimals(amount: U256, from_decimals: u8, to_decimals: u8) -> anyhow::Result<U256> {
    let scale_overflow = || {
        anyhow::anyhow!(
            "Scale to convert from {from_decimals} to {to_decimals} decimals must not be bigger than 2^256 - 1"
        )
    };
    match from_decimals.cmp(&to_decimals) {
        Ordering::Greater => {
            let divisor = U256::from(10u64)
                .checked_pow(U256::from(from_decimals - to_decimals))
                .ok_or_else(scale_overflow)?;
            Ok(amount / divisor) // exponentiation cannot be 0
        }
        Ordering::Less => {
            let multiplier = U256::from(10u64)
                .checked_pow(U256::from(to_decimals - from_decimals))
                .ok_or_else(scale_overflow)?;
            amount.checked_mul(multiplier).ok_or_else(|| {
                anyhow::anyhow!(
                    "Result of scaling {amount} by {multiplier} must not be bigger than 2^256 - 1"
                )
            })
        }
        Ordering::Equal => Ok(amount),
    }
}

/// Returns (local, remote) decimals for scaling the token for sythetic tokens
/// or (1, 1) for other kinds, thus should only be used for conversion purposes.
fn conversion_decimals(token: &StoredTokenKind) -> (u8, u8) {
    match token {
        StoredTokenKind::Synthetic {
            local_decimals,
            remote_decimals,
            ..
        } => (*local_decimals, *remote_decimals),
        _ => (1, 1),
    }
}

impl StoredTokenKind {
    /// Scales the amount to account for the differences in decimals when sending to a destination chain.
    pub fn outbound_amount(&self, local_amount: Amount) -> anyhow::Result<[u8; 32]> {
        let amount = U256::try_from(local_amount.0).unwrap();
        let (local_decimals, remote_decimals) = conversion_decimals(self);
        convert_decimals(amount, local_decimals, remote_decimals).map(|res| res.to_be_bytes())
    }

    /// Scales the amount to account for the differences in decimals when receiving a message.
    pub fn inbound_amount(&self, amount: [u8; 32]) -> anyhow::Result<Amount> {
        let amount = U256::from_be_bytes(amount);
        let (local_decimals, remote_decimals) = conversion_decimals(self);
        let scaled = convert_decimals(amount, remote_decimals, local_decimals)?;

        Ok(Amount(scaled.try_into().map_err(|_| {
            anyhow::anyhow!("Amount may not exceed 2^128 - 1 after scaling")
        })?))
    }
}

impl TokenKind {
    /// Returns the token ID and kind for the token kind.
    pub fn id_and_kind(&self) -> (HexHash, TokenKindId) {
        match self {
            TokenKind::Synthetic {
                remote_token_id, ..
            } => (*remote_token_id, TokenKindId::Synthetic),
            TokenKind::Collateral { token, .. } => {
                ((*token.as_bytes()).into(), TokenKindId::Collateral)
            }
            TokenKind::Native => (
                (*sov_bank::config_gas_token_id().as_bytes()).into(),
                TokenKindId::Native,
            ),
        }
    }
}

/// Warp route identifier i.e. its hyperlane address.
pub type WarpRouteId = HexHash;

/// A rate limiter implementing "token bucket" algorithm.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Copy,
    Clone,
    Eq,
)]
pub struct RateLimiter {
    /// Upper limit of transferrable tokens for the route.
    max_transferrable_tokens: Amount,
    /// Current transferrable tokens limit for the route.
    current_transferrable_tokens: Amount,
    /// Limit replenishment per slot for the route.
    limit_replenishment_per_slot: Amount,
    /// A last slot at which route limits were replenished.
    last_seen_visible_slot: VisibleSlotNumber,
}

impl RateLimiter {
    /// Creates a new "token bucket" rate limiter with given max limit and per slot replenishment.
    ///
    /// Initial limit will be set to a value of single replenishment and gradually
    /// raise at each slot until it reaches total limit. Each transfer will lower the current
    /// limit.
    ///
    /// The implementation slightly divereges from a solidity one. Ethereum implementation
    /// discretizes replenishment using blocks timestamps, in seconds, and always replenishes
    /// a route limits fully within 24 hours. Since each rollup can choose different DA, and they
    /// can have different or even variable block times, this implementation discretizes
    /// replenishment using slot numbers, and requires configuring a fixed replenishment value per
    /// slot.
    // NOTE: relevant ethereum impl
    //  common: <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/a47deffbc1aee90df9d940a28c790b12802b5c4b/solidity/contracts/libs/RateLimited.sol#L26>
    //  inbound: <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/a47deffbc1aee90df9d940a28c790b12802b5c4b/solidity/contracts/isms/warp-route/RateLimitedIsm.sol#L64>
    //  outbound: <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/a47deffbc1aee90df9d940a28c790b12802b5c4b/solidity/contracts/hooks/warp-route/RateLimitedHook.sol#L29>
    pub fn new<VR: VersionReader>(
        transferrable_tokens_limit: Amount,
        limit_replenishment_per_slot: Amount,
        version_reader: &VR,
    ) -> Self {
        Self {
            max_transferrable_tokens: transferrable_tokens_limit,
            current_transferrable_tokens: limit_replenishment_per_slot,
            limit_replenishment_per_slot,
            last_seen_visible_slot: version_reader.current_visible_slot_number(),
        }
    }

    /// Updates current limit and seen slot number.
    fn replenish(&mut self, visible_slot: VisibleSlotNumber) {
        self.current_transferrable_tokens = self.current_limit_with_replenishment(visible_slot);
        self.last_seen_visible_slot = visible_slot;
    }

    /// Updates the limits based on current visible slot number and the transferred amount.
    ///
    /// Returns error if transfer exceeds current limit.
    pub fn on_transfer(
        &mut self,
        transferred: Amount,
        visible_slot: VisibleSlotNumber,
    ) -> anyhow::Result<()> {
        self.replenish(visible_slot);

        // check and substract the transfer
        self.current_transferrable_tokens = self
            .current_transferrable_tokens
            .checked_sub(transferred)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Transfer of {} exceeds current limit {}",
                    transferred,
                    self.current_transferrable_tokens
                )
            })?;

        Ok(())
    }

    /// Get the current limit of the warp route, taking into account slots delta since last
    /// replenishment.
    pub fn current_limit_with_replenishment(&self, visible_slot: VisibleSlotNumber) -> Amount {
        // if route was already updated this slot or is full exit early
        if self.last_seen_visible_slot >= visible_slot
            || self.current_transferrable_tokens == self.max_transferrable_tokens
        {
            self.current_transferrable_tokens
        } else {
            let slots_since_last_update = visible_slot.delta(*self.last_seen_visible_slot);
            let replenish_amount = self
                .limit_replenishment_per_slot
                .saturating_mul(slots_since_last_update.into());

            self.max_transferrable_tokens.min(
                self.current_transferrable_tokens
                    .saturating_add(replenish_amount),
            )
        }
    }

    /// Get current limit replenishment per slot.
    pub fn limit_replenishment_per_slot(&self) -> Amount {
        self.limit_replenishment_per_slot
    }

    /// Replenishes current limit and updates replenishment per slot.
    pub fn update_limit_replenishment_per_slot(
        &mut self,
        new_replenishment: Amount,
        visible_slot: VisibleSlotNumber,
    ) {
        self.replenish(visible_slot);
        self.limit_replenishment_per_slot = new_replenishment;
    }

    /// Get current limit replenishment per slot.
    pub fn max_limit(&self) -> Amount {
        self.max_transferrable_tokens
    }

    /// Updates the upper limit of transferrable tokens, truncating current limit if necessary..
    pub fn update_max_limit(&mut self, new_limit: Amount) {
        self.max_transferrable_tokens = new_limit;
        self.current_transferrable_tokens = self.current_transferrable_tokens.min(new_limit);
    }
}

/// The authority that can modify the route.
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Clone,
    JsonSchema,
    UniversalWallet,
    Eq,
)]
#[schemars(bound = "S: Spec", rename = "Admin")]
pub enum Admin<S: Spec> {
    /// No admin - the route is immutable.
    None,
    /// Allow the specified address to modify the route. This is extremely insecure,
    /// but it seems to be common practice in Hyperlane.
    InsecureOwner(S::Address),
}

/// Represents the warp route instance.
#[derive(
    borsh::BorshDeserialize, borsh::BorshSerialize, Serialize, Deserialize, Debug, PartialEq, Clone,
)]
#[serde(bound = "S: Spec")]
pub struct WarpRouteInstance<S: Spec> {
    /// The source of the token.
    pub token_source: StoredTokenKind,
    /// The authority that can modify the route, if any.
    pub admin: Admin<S>,
    /// The ISM for this route.
    pub ism: Ism,
    /// The destination domains that are enrolled in this route.
    pub enrolled_destinations: Vec<u32>,
    /// Inbound transfers rate limiter.
    pub inbound_rate_limiter: RateLimiter,
    /// Outbound transfers rate limiter.
    pub outbound_rate_limiter: RateLimiter,
}

#[derive(
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Debug,
    PartialEq,
    Clone,
    Eq,
    JsonSchema,
)]
/// A key for a router, consisting of route ID and destination domain.
pub struct RouterKey {
    /// The address of the router.
    pub route_id: WarpRouteId,
    /// The domain of the router.
    pub remote_domain: u32,
}

impl std::fmt::Display for RouterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.route_id, self.remote_domain)
    }
}

impl std::str::FromStr for RouterKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(':');
        let route_id = parts
            .next()
            .ok_or(anyhow::anyhow!(
                "Invalid router key: missing separator token `:`"
            ))?
            .parse()?;
        let destination_domain = parts
            .next()
            .ok_or(anyhow::anyhow!(
                "Invalid router key: missing destination domain"
            ))?
            .parse()?;
        anyhow::ensure!(
            parts.next().is_none(),
            "Invalid router key: Too many separator tokens (`:`)"
        );
        Ok(RouterKey {
            route_id,
            remote_domain: destination_domain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equal_decimals_conversion() {
        for (from_dec, to_dec) in [(0, 0), (1, 1), (8, 8), (255, 255)] {
            assert_eq!(
                U256::from(1),
                convert_decimals(U256::from(1), from_dec, to_dec).unwrap()
            );
        }
    }

    #[test]
    fn test_scaling_back_and_forth() {
        let amount = U256::from(1000);
        let from_dec = 3;
        let to_dec = 5;
        let scaled = convert_decimals(amount, from_dec, to_dec).unwrap();

        assert_eq!(U256::from(100000), scaled);
        assert_eq!(amount, convert_decimals(scaled, to_dec, from_dec).unwrap());

        // loss of precision
        let amount = U256::from(12345);
        let from_dec = 4;
        let to_dec = 2;
        let scaled = convert_decimals(amount, from_dec, to_dec).unwrap();

        assert_eq!(
            U256::from(12300),
            convert_decimals(scaled, to_dec, from_dec).unwrap()
        );
    }

    #[test]
    fn test_scaling_overflows() {
        // this shouldn't overflow
        convert_decimals(U256::from(1), 1, 78).unwrap();
        convert_decimals(U256::from(1), 78, 1).unwrap();
        // but this makes scale too big
        convert_decimals(U256::from(1), 1, 79).unwrap_err();
        convert_decimals(U256::from(1), 79, 1).unwrap_err();
        // and this makes result after scaling too big
        convert_decimals(U256::from(10), 1, 78).unwrap_err();
    }
}
