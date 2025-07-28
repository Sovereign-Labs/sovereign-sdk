use core::str::FromStr;

use anyhow::{anyhow, bail, Context as _};
use schemars::JsonSchema;
use sov_bank::Amount;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::Spec;

use crate::types::Domain;

/// Oracle data used to calculate required gas.
#[derive(
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    JsonSchema,
    UniversalWallet,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
)]
pub struct ExchangeRateAndGasPrice {
    /// Gas price.
    pub gas_price: Amount,
    /// Token exchange rate, calculated as local gas token price / remote gas token price.
    ///
    /// Relayer is responsible to multiply token_rate_exchange by `TOKEN_EXCHANGE_RATE_SCALE`.
    pub token_exchange_rate: u128,
}

/// Domain Default Gas used in `CallMessage::SetRelayerConfig`.
#[derive(
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    Clone,
    PartialEq,
    Eq,
    JsonSchema,
    UniversalWallet,
)]
pub struct DomainDefaultGas {
    /// Domain.
    pub domain: Domain,
    /// Default gas.
    pub default_gas: Amount,
}

/// Domain Oracle Data used in `CallMessage::SetRelayerConfig`.
#[derive(
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    Clone,
    PartialEq,
    Eq,
    JsonSchema,
    UniversalWallet,
)]
pub struct DomainOracleData {
    /// Domain.
    pub domain: Domain,
    /// Oracle data value.
    pub data_value: ExchangeRateAndGasPrice,
}

/// Composite Key: relayer + domain.
///
/// Used to store relayer data per domain.
#[derive(
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    serde::Serialize,
    serde::Deserialize,
    derive_more::Display,
    Debug,
    Clone,
    PartialEq,
    Eq,
    JsonSchema,
    UniversalWallet,
)]
#[schemars(bound = "S: Spec", rename = "RelayerWithDomainKey")]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[display(r#"{}/{}"#, self.relayer, self.domain)]
pub struct RelayerWithDomainKey<S: Spec> {
    /// Relayer.
    pub relayer: S::Address,
    /// Domain (i.e. chain id on hyperlane).
    pub domain: Domain,
}

impl<S: Spec> RelayerWithDomainKey<S> {
    /// Create composite key.
    pub fn new(relayer: S::Address, domain: Domain) -> Self {
        Self { relayer, domain }
    }
}

impl<S: Spec> FromStr for RelayerWithDomainKey<S> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Pick last index of '/' since address could contain it
        let (relayer, dest_domain) = s.rsplit_once('/').ok_or_else(|| {
            anyhow!(
                "Invalid format: expected 'relayer/dest_domain', got '{}'",
                s
            )
        })?;

        if relayer.is_empty() {
            bail!("Relayer part cannot be empty");
        }
        if dest_domain.is_empty() {
            bail!("Domain part cannot be empty");
        }

        let relayer = S::Address::from_str(relayer)
            .map_err(|_| anyhow!("could not parse address from {}", relayer))?;
        let dest_domain = dest_domain
            .parse()
            .context("Failed to parse destination domain")?;

        Ok(Self {
            relayer,
            domain: dest_domain,
        })
    }
}
