use anyhow::{bail, Result};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::{GenesisState, Module, Spec};

use crate::token::{unique_holders, Token};
use crate::utils::TokenHolderRef;
use crate::{config_gas_token_id, Amount, Bank, TokenId};

/// Initial configuration for sov-bank module.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, JsonSchema)]
#[serde(bound = "S::Address: Serialize + DeserializeOwned")]
#[schemars(bound = "S: Spec", rename = "BankConfig")]
pub struct BankConfig<S: Spec> {
    /// Configuration for the gas token
    pub gas_token_config: GasTokenConfig<S>,
    /// A list of configurations for any other tokens to create at genesis
    pub tokens: Vec<TokenConfig<S>>,
}

/// [`TokenConfig`] specifies a configuration used when generating a token for the bank
/// module.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, JsonSchema, derive_more::Display)]
#[serde(bound = "S::Address: Serialize + DeserializeOwned")]
#[display("{:?}", self)]
#[schemars(bound = "S: Spec", rename = "TokenConfig")]
pub struct TokenConfig<S: Spec> {
    /// The name of the token.
    pub token_name: String,
    /// The decimal places of the token.
    pub token_decimals: Option<u8>,
    /// Predetermined ID of the token. Allowed only for genesis tokens.
    pub token_id: TokenId,
    /// A vector of tuples containing the initial addresses and balances (as u128)
    pub address_and_balances: Vec<(S::Address, Amount)>,
    /// The addresses that are authorized to mint the token.
    pub admins: Vec<S::Address>,
    /// The supply cap of the token, if any.
    pub supply_cap: Option<Amount>,
}

/// [`GasTokenConfig`] specifies a configuration for the rollup's gas token.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, JsonSchema)]
#[serde(bound = "S::Address: Serialize + DeserializeOwned")]
#[schemars(bound = "S: Spec", rename = "GasTokenConfig")]
pub struct GasTokenConfig<S: Spec> {
    /// The name of the token.
    pub token_name: String,
    /// The decimal places of the token.
    pub token_decimals: Option<u8>,
    /// A vector of tuples containing the initial addresses and balances (as u128)
    pub address_and_balances: Vec<(S::Address, Amount)>,
    /// The addresses that are authorized to mint the token.
    pub admins: Vec<S::Address>,
    /// The supply cap of the token, if any.
    pub supply_cap: Option<Amount>,
}

impl<S: Spec> From<GasTokenConfig<S>> for TokenConfig<S> {
    fn from(gas_token_config: GasTokenConfig<S>) -> Self {
        TokenConfig {
            token_name: gas_token_config.token_name,
            token_decimals: gas_token_config.token_decimals,
            token_id: crate::config_gas_token_id(),
            address_and_balances: gas_token_config.address_and_balances,
            admins: gas_token_config.admins,
            supply_cap: gas_token_config.supply_cap,
        }
    }
}

impl<S: Spec> core::fmt::Display for GasTokenConfig<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let address_and_balances = self
            .address_and_balances
            .iter()
            .map(|(address, balance)| format!("({address}, {balance})"))
            .collect::<Vec<String>>()
            .join(", ");

        write!(
            f,
            "TokenConfig {{ token_name: {}, token_id: {}, address_and_balances: [{}] }}",
            self.token_name,
            config_gas_token_id(),
            address_and_balances,
        )
    }
}

impl<S: Spec> Bank<S> {
    /// Init an instance of the bank module from the configuration `config`.
    /// For each token in the `config`, calls the [`Token::create`] function to create
    /// the token. Upon success, updates the token set if the token ID doesn't already exist.
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        let gas_token_config: TokenConfig<S> = config.gas_token_config.clone().into();
        tracing::debug!(token_id = %config_gas_token_id(), token_name = %gas_token_config.token_name, "Gas token");
        for token_config in std::iter::once(&gas_token_config).chain(config.tokens.iter()) {
            let token_id = &token_config.token_id;
            tracing::debug!(
                %token_config,
                token_id = %token_id,
                "Genesis of the token");

            let admins = token_config
                .admins
                .iter()
                .map(|minter| TokenHolderRef::<'_, S>::from(&minter))
                .collect::<Vec<_>>();

            let mut total_supply = Amount::ZERO;
            for (address, balance) in token_config.address_and_balances.iter() {
                let addr = TokenHolderRef::<'_, S>::from(&address);
                tracing::trace!(%address, %balance, %token_id, "Genesis balance of address");
                self.balances.set(&(addr, token_id), balance, state)?;
                total_supply = total_supply
                    .checked_add(*balance)
                    .ok_or(anyhow::anyhow!("Overflowed token supply!"))?;
            }

            let supply_cap = token_config.supply_cap.unwrap_or(Amount::MAX);
            if total_supply > supply_cap {
                anyhow::bail!(
                    "Error: The total_supply: {total_supply} exceeds supply_cap: {supply_cap}"
                );
            }

            let admins = unique_holders(&admins);
            let token = Token::<S> {
                name: token_config.token_name.to_owned(),
                total_supply,
                supply_cap,
                admins,
            };

            if self.tokens.get(token_id, state)?.is_some() {
                bail!("token ID {} already exists", token_config.token_id);
            }

            self.tokens.set(token_id, &token, state)?;
            tracing::debug!(
                token_name = %token.name,
                token_id = %token_id,
                "Token has been created"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_modules_api::prelude::serde_json;
    use sov_modules_api::{AddressBech32, Spec};
    use sov_test_utils::TestSpec;

    use super::*;
    use crate::get_token_id;

    #[test]
    fn test_config_serialization() {
        let sender_address: <TestSpec as Spec>::Address =
            AddressBech32::from_str("sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv")
                .unwrap()
                .into();
        let token_id =
            TokenId::from_str("token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7")
                .unwrap();

        let config = BankConfig::<TestSpec> {
            gas_token_config: GasTokenConfig {
                token_name: "sov-gas-token".to_owned(),
                token_decimals: None,
                address_and_balances: vec![(sender_address, Amount::new(100_000_000))],
                admins: vec![sender_address],
                supply_cap: None,
            },
            tokens: vec![TokenConfig {
                token_name: "sov-demo-token".to_owned(),
                token_decimals: Some(6),
                token_id,
                address_and_balances: vec![(sender_address, Amount::new(1000))],
                admins: vec![sender_address],
                supply_cap: None,
            }],
        };

        let data = r#"
        {
            "gas_token_config": {
                "token_name":"sov-gas-token",
                "address_and_balances":[["sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv","100000000"]],
                "admins":["sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv"]
            },
            "tokens":[
                {
                    "token_name":"sov-demo-token",
                    "token_decimals": 6,
                    "token_id": "token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7",
                    "address_and_balances":[["sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv","1000"]],
                    "admins":["sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv"]
                }
            ]
        }"#;

        let parsed_config: BankConfig<TestSpec> = serde_json::from_str(data).unwrap();

        assert_eq!(config, parsed_config);
    }

    #[test]
    fn test_token_id() {
        let originator: <TestSpec as Spec>::Address =
            AddressBech32::from_str("sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv")
                .unwrap()
                .into();

        let gas_token_id = get_token_id::<TestSpec>("sov-gas-token", Some(6), &originator);
        assert_eq!(
            gas_token_id,
            TokenId::from_str("token_1kvvxstujsan95u2jfcar0t5jzy6u5xy56am0xz4jkazpn28egurqy5gmnl")
                .unwrap()
        );
    }
}
