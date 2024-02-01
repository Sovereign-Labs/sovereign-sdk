//! An implementation of the eth gas price oracle, used for providing gas price estimates based on
//! previous blocks.

// Adopted from: https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc/src/eth/gas_oracle.rs

use reth_primitives::constants::GWEI_TO_WEI;
use reth_primitives::{H256, U256, U64};
use reth_rpc_types::BlockTransactions;
use serde::{Deserialize, Serialize};
use sov_evm::{EthApiError, EthResult, Evm, RpcInvalidTransactionError};
use sov_modules_api::WorkingSet;
use tokio::sync::Mutex;
use tracing::warn;

use super::cache::BlockCache;

/// The number of transactions sampled in a block
pub const SAMPLE_NUMBER: u32 = 3;

/// The default maximum gas price to use for the estimate
pub const DEFAULT_MAX_PRICE: U256 = U256::from_limbs([500_000_000_000u64, 0, 0, 0]);

/// The default minimum gas price, under which the sample will be ignored
pub const DEFAULT_IGNORE_PRICE: U256 = U256::from_limbs([2u64, 0, 0, 0]);

/// Settings for the gas price oracle configured by node operators
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GasPriceOracleConfig {
    /// The number of populated blocks to produce the gas price estimate
    pub blocks: u32,

    /// The percentile of gas prices to use for the estimate
    pub percentile: u32,

    /// The maximum number of headers to keep in the cache
    pub max_header_history: u32,

    /// The maximum number of blocks for estimating gas price
    pub max_block_history: u64,

    /// The default gas price to use if there are no blocks to use
    pub default: Option<U256>,

    /// The maximum gas price to use for the estimate
    pub max_price: Option<U256>,

    /// The minimum gas price, under which the sample will be ignored
    pub ignore_price: Option<U256>,
}

impl Default for GasPriceOracleConfig {
    fn default() -> Self {
        GasPriceOracleConfig {
            blocks: 20,
            percentile: 60,
            max_header_history: 1024,
            max_block_history: 1024,
            default: None,
            max_price: Some(DEFAULT_MAX_PRICE),
            ignore_price: Some(DEFAULT_IGNORE_PRICE),
        }
    }
}

impl GasPriceOracleConfig {
    /// Creating a new gpo config with blocks, ignoreprice, maxprice and percentile
    pub fn new(
        blocks: Option<u32>,
        ignore_price: Option<u64>,
        max_price: Option<u64>,
        percentile: Option<u32>,
    ) -> Self {
        Self {
            blocks: blocks.unwrap_or(20),
            percentile: percentile.unwrap_or(60),
            max_header_history: 1024,
            max_block_history: 1024,
            default: None,
            max_price: max_price.map(U256::from).or(Some(DEFAULT_MAX_PRICE)),
            ignore_price: ignore_price.map(U256::from).or(Some(DEFAULT_IGNORE_PRICE)),
        }
    }
}

/// Calculates a gas price depending on recent blocks.
pub struct GasPriceOracle<C: sov_modules_api::Context> {
    /// The type used to get block and tx info
    provider: Evm<C>,
    /// The config for the oracle
    oracle_config: GasPriceOracleConfig,
    /// The latest calculated price and its block hash
    last_price: Mutex<GasPriceOracleResult>,
    /// Cache
    cache: BlockCache<C>,
}

impl<C: sov_modules_api::Context> GasPriceOracle<C> {
    /// Creates and returns the [GasPriceOracle].
    pub fn new(provider: Evm<C>, mut oracle_config: GasPriceOracleConfig) -> Self {
        // sanitize the percentile to be less than 100
        if oracle_config.percentile > 100 {
            warn!(prev_percentile = ?oracle_config.percentile, "Invalid configured gas price percentile, assuming 100.");
            oracle_config.percentile = 100;
        }

        let max_header_history = oracle_config.max_header_history;

        Self {
            provider: provider.clone(),
            oracle_config,
            last_price: Default::default(),
            cache: BlockCache::<C>::new(max_header_history, provider),
        }
    }

    /// Suggests a gas price estimate based on recent blocks, using the configured percentile.
    pub async fn suggest_tip_cap(&self, working_set: &mut WorkingSet<C>) -> EthResult<U256> {
        let header = &self
            .provider
            .get_block_by_number(None, None, working_set)
            .unwrap()
            .unwrap()
            .header;

        let mut last_price = self.last_price.lock().await;

        // if we have stored a last price, then we check whether or not it was for the same head
        if last_price.block_hash == header.hash.unwrap() {
            return Ok(last_price.price);
        }

        // if all responses are empty, then we can return a maximum of 2*check_block blocks' worth
        // of prices
        //
        // we only return more than check_block blocks' worth of prices if one or more return empty
        // transactions
        let mut current_hash = header.hash.unwrap();
        let mut results = Vec::new();
        let mut populated_blocks = 0;

        let header_number = convert_u256_to_u64(header.number.unwrap());

        // we only check a maximum of 2 * max_block_history, or the number of blocks in the chain
        let max_blocks = if self.oracle_config.max_block_history * 2 > header_number {
            header_number
        } else {
            self.oracle_config.max_block_history * 2
        };

        for _ in 0..max_blocks {
            let (parent_hash, block_values) = self
                .get_block_values(current_hash, SAMPLE_NUMBER as usize, working_set)
                .await?
                .ok_or(EthApiError::UnknownBlockNumber)?;

            if block_values.is_empty() {
                results.push(U256::from(last_price.price));
            } else {
                results.extend(block_values);
                populated_blocks += 1;
            }

            // break when we have enough populated blocks
            if populated_blocks >= self.oracle_config.blocks {
                break;
            }

            current_hash = parent_hash;
        }

        // sort results then take the configured percentile result
        let mut price = last_price.price;
        if !results.is_empty() {
            results.sort_unstable();
            price = *results
                .get((results.len() - 1) * self.oracle_config.percentile as usize / 100)
                .expect("gas price index is a percent of nonzero array length, so a value always exists; qed");
        }

        // constrain to the max price
        if let Some(max_price) = self.oracle_config.max_price {
            if price > max_price {
                price = max_price;
            }
        }

        *last_price = GasPriceOracleResult {
            block_hash: header.hash.unwrap(),
            price,
        };

        Ok(price)
    }

    /// Get the `limit` lowest effective tip values for the given block. If the oracle has a
    /// configured `ignore_price` threshold, then tip values under that threshold will be ignored
    /// before returning a result.
    ///
    /// If the block cannot be found, then this will return `None`.
    ///
    /// This method also returns the parent hash for the given block.
    async fn get_block_values(
        &self,
        block_hash: H256,
        limit: usize,
        working_set: &mut WorkingSet<C>,
    ) -> EthResult<Option<(H256, Vec<U256>)>> {
        // check the cache (this will hit the disk if the block is not cached)
        let block = match self.cache.get_block(block_hash, working_set)? {
            Some(block) => block,
            None => return Ok(None),
        };

        // sort the transactions by effective tip
        // but first filter those that should be ignored

        // get the transactions (block.transactions is a enum but we only care about the 2nd arm)
        let txs = match &block.transactions {
            BlockTransactions::Full(txs) => txs,
            _ => return Ok(None),
        };

        let mut txs = txs
            .iter()
            .filter(|tx| {
                if let Some(ignore_under) = self.oracle_config.ignore_price {
                    let effective_gas_tip = effective_gas_tip(tx, block.header.base_fee_per_gas);
                    if effective_gas_tip < Some(ignore_under) {
                        return false;
                    }
                }

                // check if coinbase
                let sender = tx.from;
                sender != block.header.miner
            })
            // map all values to effective_gas_tip because we will be returning those values
            // anyways
            .map(|tx| effective_gas_tip(tx, block.header.base_fee_per_gas))
            .collect::<Vec<_>>();

        // now do the sort
        txs.sort_unstable();

        // fill result with the top `limit` transactions
        let mut final_result = Vec::with_capacity(limit);
        for tx in txs.iter().take(limit) {
            // a `None` effective_gas_tip represents a transaction where the max_fee_per_gas is
            // less than the base fee
            let effective_tip = tx.ok_or(RpcInvalidTransactionError::FeeCapTooLow)?;
            final_result.push(U256::from(effective_tip));
        }

        Ok(Some((block.header.parent_hash, final_result)))
    }
}

/// Stores the last result that the oracle returned
#[derive(Debug, Clone)]
pub struct GasPriceOracleResult {
    /// The block hash that the oracle used to calculate the price
    pub block_hash: H256,
    /// The price that the oracle calculated
    pub price: U256,
}

impl Default for GasPriceOracleResult {
    fn default() -> Self {
        Self {
            block_hash: H256::zero(),
            price: U256::from(GWEI_TO_WEI),
        }
    }
}

// Adopted from: https://github.com/paradigmxyz/reth/blob/main/crates/primitives/src/transaction/mod.rs#L297
fn effective_gas_tip(
    transaction: &reth_rpc_types::Transaction,
    base_fee: Option<U256>,
) -> Option<U256> {
    let priority_fee_or_price = U256::from(match transaction.transaction_type {
        Some(U64([2])) => transaction.max_priority_fee_per_gas.unwrap(),
        _ => transaction.gas_price.unwrap(),
    });

    if let Some(base_fee) = base_fee {
        let max_fee_per_gas = U256::from(match transaction.transaction_type {
            Some(U64([2])) => transaction.max_fee_per_gas.unwrap(),
            _ => transaction.gas_price.unwrap(),
        });

        if max_fee_per_gas < base_fee {
            None
        } else {
            let effective_max_fee = max_fee_per_gas - base_fee;
            Some(std::cmp::min(effective_max_fee, priority_fee_or_price))
        }
    } else {
        Some(priority_fee_or_price)
    }
}

/// Takes only 8 least significant bytes
fn convert_u256_to_u64(u256: U256) -> u64 {
    let bytes: [u8; 32] = u256.to_be_bytes();
    // 32 - 24 = 8, so it should always fit into destination array.
    // Unless allocation or something weird
    let bytes: [u8; 8] = bytes[24..].try_into().unwrap();
    u64::from_be_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use proptest::arbitrary::any;
    use proptest::proptest;
    use reth_primitives::constants::GWEI_TO_WEI;

    use super::*;

    #[test]
    fn max_price_sanity() {
        assert_eq!(DEFAULT_MAX_PRICE, U256::from(500_000_000_000u64));
        assert_eq!(DEFAULT_MAX_PRICE, U256::from(500 * GWEI_TO_WEI))
    }

    #[test]
    fn ignore_price_sanity() {
        assert_eq!(DEFAULT_IGNORE_PRICE, U256::from(2u64));
    }

    proptest! {

        #[test]
        fn converts_back_and_forth(input in any::<u64>()) {
            let mut bytes: [u8; 32] = [0; 32];
            for (i, b) in input.to_be_bytes().into_iter().enumerate() {
                let idx = 24 + i;
                bytes[idx] = b;
            }


            let u256 = U256::from_be_slice(&bytes);
            let output = convert_u256_to_u64(u256);

            assert_eq!(input, output);
        }

        #[test]
        fn convert_u256_to_u64_doesnt_panic(input in any::<[u8; 32]>()) {
            let u256 = U256::from_be_slice(&input);
            let _output = convert_u256_to_u64(u256);
        }
    }
}
