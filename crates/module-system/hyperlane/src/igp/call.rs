use std::collections::HashMap;

use anyhow::{bail, Context as _, Result};
use schemars::JsonSchema;
use sov_bank::{Amount, IntoPayable};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, EventEmitter, SafeVec, Spec};
use strum::{EnumDiscriminants, EnumIs, VariantArray};

use super::event::Event;
use super::types::{
    DomainDefaultGas, DomainOracleData, ExchangeRateAndGasPrice, RelayerWithDomainKey,
};
use super::{native_gas_coins, InterchainGasPaymaster};
use crate::types::Domain;

/// The maximum number of supported domains per relayer.
pub const MAX_DOMAINS_PER_RELAYER_COUNT: usize = 100;

/// InterchainGasPaymaster CallMessage
#[derive(
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    JsonSchema,
    EnumDiscriminants,
    EnumIs,
    UniversalWallet,
)]
#[strum_discriminants(derive(VariantArray, EnumIs))]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "CallMessage")]
pub enum CallMessage<S: Spec> {
    /// Set or update config for relayer (sender).
    ///
    /// This could be used to clear values too.
    SetRelayerConfig {
        /// oracle data per domain.
        domain_oracle_data: SafeVec<DomainOracleData, MAX_DOMAINS_PER_RELAYER_COUNT>,
        /// Custom default gas per domain.
        domain_default_gas: SafeVec<DomainDefaultGas, MAX_DOMAINS_PER_RELAYER_COUNT>,
        /// Default gas used if custom one is not set.
        default_gas: Amount,
        /// Beneficiary who can claim relayer rewards.
        beneficiary: Option<S::Address>,
    },
    /// Update oracle data for relayer (sender)
    UpdateOracleData {
        /// Domain or destination domain (i.e. chain id in hyperlane).
        domain: Domain,
        /// Oracle data.
        ///
        /// Relayer is responsible to multiple token_rate_exchange by `TOKEN_EXCHANGE_RATE_SCALE`.
        oracle_data: ExchangeRateAndGasPrice,
    },
    /// Beneficiary (sender) claim all relayer rewards.
    ClaimRewards {
        /// Relayer to transfer tokens from.
        relayer_address: S::Address,
    },
}

impl<S: Spec> InterchainGasPaymaster<S> {
    /// Upsert relayer's (sender) `InterchainGasPaymaster` gas data and set beneficiary who can claim relayer rewards.
    ///
    /// Emits `SetRelayerConfig` and `OracleDataUpdated` events
    pub fn set_relayer_config(
        &mut self,
        domain_oracle_data: HashMap<Domain, ExchangeRateAndGasPrice>,
        domain_default_gas: HashMap<Domain, Amount>,
        default_gas: Amount,
        beneficiary: Option<S::Address>,
        context: &Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> Result<()> {
        let relayer = context.sender();

        if default_gas == Amount::ZERO {
            bail!("Default gas must be nonzero");
        }

        self.relayer_default_gas
            .set(relayer, &default_gas, state)
            .context("failed to set relayer default gas")?;

        for (domain, oracle_data) in domain_oracle_data {
            if oracle_data.gas_price == Amount::ZERO || oracle_data.token_exchange_rate == 0 {
                bail!(
                    "Incorrect oracle data (gas_price: {}, exchange rate: {}) for domain {}. Gas price and exchange rate must be nonzero",
                    oracle_data.gas_price,
                    oracle_data.token_exchange_rate,
                    domain
                );
            }

            let key = RelayerWithDomainKey::new(relayer.clone(), domain);

            self.domain_oracle_data
                .set(&key, &oracle_data, state)
                .context("failed to set relayer oracle data")?;

            self.emit_event(
                state,
                Event::OracleDataUpdated {
                    relayer: relayer.clone(),
                    domain,
                    oracle_data,
                },
            );
        }

        for (domain, amount) in domain_default_gas.iter() {
            if amount == &Amount::ZERO {
                bail!("Default gas for domain {domain} must be nonzero");
            }

            let key = RelayerWithDomainKey::new(relayer.clone(), *domain);

            self.domain_default_gas
                .set(&key, amount, state)
                .context("set relayer domain default gas")?;
        }

        self.beneficiary
            .set(relayer, &beneficiary, state)
            .context("failed to set relayer benefeciary")?;

        self.emit_event(
            state,
            Event::RelayerConfigSet {
                relayer: relayer.clone(),
                domain_custom_gas: domain_default_gas,
                default_gas,
                beneficiary,
            },
        );
        Ok(())
    }

    /// Update relayer's (sender) oracle data (gas price & token_exchange_rate).
    ///
    /// Emits `OracleDataUpdated` event.
    pub fn update_oracle_value(
        &mut self,
        domain: Domain,
        oracle_data: ExchangeRateAndGasPrice,
        context: &Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> Result<()> {
        let key = RelayerWithDomainKey::new(context.sender().clone(), domain);
        self.domain_oracle_data
            .set(&key, &oracle_data, state)
            .context("set relayer oracle data")?;
        self.emit_event(
            state,
            Event::OracleDataUpdated {
                relayer: key.relayer,
                domain: key.domain,
                oracle_data,
            },
        );
        Ok(())
    }

    /// Beneficiary (sender) claim of relayer rewards.
    ///
    /// If beneficiary is `None` or sender doesnt match - error will be returned.
    ///
    /// Emits `RewardsClaimed` event.
    pub fn claim(
        &mut self,
        relayer: S::Address,
        context: &Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> Result<()> {
        let beneficiary = self
            .beneficiary
            .get(&relayer, state)
            .context("get relayer beneficiary")?
            .ok_or(anyhow::anyhow!("beneficiary not found"))?;

        let Some(beneficiary) = beneficiary else {
            anyhow::bail!("Access denied");
        };

        let sender = context.sender();
        if sender != &beneficiary {
            anyhow::bail!("Access denied");
        }

        let balance = self
            .funds
            .get(&relayer, state)
            .context("get relayer funds")?
            .unwrap_or(Amount::ZERO);

        if balance == 0 {
            return Ok(());
        }

        self.bank.transfer_from(
            self.id.to_payable(),
            &beneficiary,
            native_gas_coins(balance),
            state,
        )?;

        self.funds
            .set(&relayer, &Amount::ZERO, state)
            .context("set relayer funds to 0")?;

        self.emit_event(
            state,
            Event::RewardsClaimed {
                beneficiary,
                relayer,
            },
        );

        Ok(())
    }
}
