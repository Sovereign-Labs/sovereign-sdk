//! Defines a module that handles interchain gas payments to relayers.
//!
//! <https://docs.hyperlane.xyz/docs/protocol/interchain-gas-payment>

use std::collections::HashMap;

use anyhow::{anyhow, bail, Context as _, Result};
// reference https://github.com/many-things/cw-hyperlane/blob/main/contracts/igps
use sov_bank::{config_gas_token_id, Amount, Bank, Coins};
use sov_modules_api::prelude::tracing;
use sov_modules_api::{
    Context, DaSpec, GenesisState, HexString, Module, ModuleId, ModuleInfo, ModuleRestApi, SafeVec,
    Spec, StateMap, StateReader,
};
use sov_state::User;

use crate::types::Domain;

mod call;
mod event;
mod hooks;
mod metadata;
mod types;

pub use call::CallMessage;
pub use event::Event;
pub use metadata::IGPMetadata;
pub use types::*;

/// Scaling factor used for representing and calculating token exchange rates.
///
/// The same value as in sealevel implementation was choosen, to allow for
/// flexibility scaling between different chains. This gives 19/20 digits on the
/// left of decimal point and 19 digits on the right of it.
// See <https://github.com/hyperlane-xyz/hyperlane-monorepo/blob/d7cb7ab1f413c510c66bf8152e4cdbdcbacbc359/rust/sealevel/programs/hyperlane-sealevel-igp/src/accounts.rs#L14-L16>
pub const TOKEN_EXCHANGE_RATE_SCALE: u128 = 10u128.pow(19);

/// Interchain Gas Paymaster module
///
/// IGP by spec is maintained per relayer via custom smart contract. In our case, module should be regarded as a smart contract, so we need to map data to specific relayers.
/// In the end this module works as a wrapper similar to Recipient but selects only 1 relayer.
///
/// NOTE: All states are StateMap<RelayerAddress, $DATA>
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct InterchainGasPaymaster<S: Spec> {
    /// Module identifier.
    #[id]
    pub id: ModuleId,
    /// A mapping from relayer/domain to its oracle data.
    #[state]
    pub domain_oracle_data: StateMap<RelayerWithDomainKey<S>, ExchangeRateAndGasPrice>,
    /// A mapping from relayer/domain to default gas.
    #[state]
    pub domain_default_gas: StateMap<RelayerWithDomainKey<S>, Amount>,
    /// A mapping from relayer to default (fallback) gas.
    #[state]
    pub relayer_default_gas: StateMap<S::Address, Amount>,
    /// A mapping from relayer to its current claimable reward funds.
    #[state]
    pub funds: StateMap<S::Address, Amount>,
    /// A mapping from relayer to its beneficiary (who can claim relayer reward tokens).
    #[state]
    pub beneficiary: StateMap<S::Address, Option<S::Address>>,
    /// The Bank module.
    #[module]
    pub bank: Bank<S>,
    /// Phantom data for the specification.
    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for InterchainGasPaymaster<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = call::CallMessage<S>;
    type Event = Event<S>;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        message: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl sov_modules_api::TxState<Self::Spec>,
    ) -> anyhow::Result<()> {
        match message {
            CallMessage::ClaimRewards { relayer_address } => {
                self.claim(relayer_address, context, state)?;
            }
            CallMessage::SetRelayerConfig {
                domain_oracle_data,
                domain_default_gas,
                default_gas,
                beneficiary,
            } => self.set_relayer_config(
                vec_to_hashmap(oracle_data_to_vec(domain_oracle_data))
                    .context("domain oracle data")?,
                vec_to_hashmap(default_gas_to_vec(domain_default_gas))
                    .context("domain default gas")?,
                default_gas,
                beneficiary,
                context,
                state,
            )?,
            CallMessage::UpdateOracleData {
                domain,
                oracle_data: oracle_value,
            } => self.update_oracle_value(domain, oracle_value, context, state)?,
        }

        Ok(())
    }
}

impl<S: Spec> InterchainGasPaymaster<S> {
    /// Calculate required gas
    pub(crate) fn quote_gas_price<Accessor: StateReader<User>>(
        &self,
        key: &RelayerWithDomainKey<S>,
        fees: Amount,
        state: &mut Accessor,
    ) -> Result<Amount> {
        let ExchangeRateAndGasPrice {
            gas_price,
            token_exchange_rate,
        } = self
            .domain_oracle_data
            .get(key, state)
            .context("get relayer domain gas")?
            .ok_or(anyhow!("oracle gas not set for domain"))?;

        required_gas(fees, gas_price, token_exchange_rate)
    }

    // Get quote data (gas limit & gas required)
    //
    // On any failure fallbacks to relayer default gas
    pub(crate) fn prepare_quote(
        &self,
        key: &RelayerWithDomainKey<S>,
        metadata: &HexString,
        _context: &Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> Result<Quote> {
        let metadata = if metadata.0.len() < 66 {
            tracing::debug!("Metadata too short for IGP info, using default gas");
            IGPMetadata {
                gas_limit: self.default_gas(key, state)?,
            }
        } else {
            match IGPMetadata::deserialize(&metadata.0) {
                Ok(parsed) => parsed,
                Err(e) => {
                    tracing::warn!("Failed to parse IGP metadata: {}, using default gas", e);
                    IGPMetadata {
                        gas_limit: self.default_gas(key, state)?,
                    }
                }
            }
        };

        let gas_required = self.quote_gas_price(key, metadata.gas_limit, state)?;

        Ok(Quote {
            metadata,
            gas_required,
        })
    }

    fn default_gas(
        &self,
        key: &RelayerWithDomainKey<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> Result<Amount> {
        let amount = self
            .domain_default_gas
            .get(key, state)
            .context("get domain default gas")?;

        match amount {
            Some(amount) => Ok(amount),
            None => {
                let amount = self
                    .relayer_default_gas
                    .get(&key.relayer, state)
                    .context("get relayer default gas")?;

                match amount {
                    Some(amount) if amount.0 > 0 => Ok(amount),
                    None => bail!("default igp fee amount not set"),
                    Some(amount) => {
                        bail!("default gas amount set to an incorrect value {amount}")
                    }
                }
            }
        }
    }
}

pub(crate) struct Quote {
    pub metadata: IGPMetadata,
    pub gas_required: Amount,
}

fn native_gas_coins(amount: Amount) -> Coins {
    Coins {
        token_id: config_gas_token_id(),
        amount,
    }
}

fn default_gas_to_vec<const MAX_SIZE: usize>(
    data: SafeVec<DomainDefaultGas, MAX_SIZE>,
) -> Vec<(Domain, Amount)> {
    data.into_iter()
        .map(|r| (r.domain, r.default_gas))
        .collect()
}

fn oracle_data_to_vec<const MAX_SIZE: usize>(
    data: SafeVec<DomainOracleData, MAX_SIZE>,
) -> Vec<(Domain, ExchangeRateAndGasPrice)> {
    data.into_iter().map(|r| (r.domain, r.data_value)).collect()
}

fn vec_to_hashmap<K, V>(vec: Vec<(K, V)>) -> Result<HashMap<K, V>>
where
    K: Eq + std::hash::Hash + std::fmt::Debug,
{
    let mut map = HashMap::with_capacity(vec.len());

    for (key, value) in vec {
        if map.contains_key(&key) {
            return Err(anyhow!("Duplicate key found: {:?}", key));
        }
        map.insert(key, value);
    }

    Ok(map)
}

fn required_gas(fees: Amount, gas_price: Amount, token_exchange_rate: u128) -> Result<Amount> {
    type U256 = ruint::Uint<256, 4>;

    let fees = U256::try_from(fees.0).unwrap();
    let gas_price = U256::try_from(gas_price.0).unwrap();
    let token_exchange_rate = U256::try_from(token_exchange_rate).unwrap();
    let token_exchange_rate_scale = U256::try_from(TOKEN_EXCHANGE_RATE_SCALE).unwrap();

    let dest_gas_cost = fees * gas_price;
    let gas_required = dest_gas_cost
        .checked_mul(token_exchange_rate)
        .ok_or(anyhow!("gas required mul overflow"))?
        .checked_div(token_exchange_rate_scale)
        .ok_or(anyhow!("token exchange scale rate is 0"))?;

    Ok(Amount(gas_required.try_into().map_err(|_| {
        anyhow::anyhow!("Amount may not exceed 2^128 - 1 after scaling")
    })?))
}

#[cfg(test)]
mod tests {
    use sov_bank::Amount;

    use super::{required_gas, TOKEN_EXCHANGE_RATE_SCALE};

    #[test]
    fn required_gas_overflow_handling() {
        // overflow u128 during computation but result fits u128
        let fees = Amount::MAX;
        let gas_price = Amount(TOKEN_EXCHANGE_RATE_SCALE);
        let token_exchange_rate = 1;

        assert_eq!(
            required_gas(fees, gas_price, token_exchange_rate).unwrap(),
            Amount::MAX
        );

        // overflow u128 in result
        let fees = Amount::MAX;
        let gas_price = Amount::MAX;
        let token_exchange_rate = 1;

        required_gas(fees, gas_price, token_exchange_rate).unwrap_err();

        // completely overflow u256
        let fees = Amount::MAX;
        let gas_price = Amount::MAX;
        let token_exchange_rate = u128::MAX;

        required_gas(fees, gas_price, token_exchange_rate).unwrap_err();
    }
}
