//! An implementation of the eth gas price oracle, used for providing gas price estimates based on
//! previous blocks.

// Adopted from: https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc/src/eth/gas_oracle.rs

use alloy_consensus::BlockHeader;
use alloy_primitives::{B256, U256};
use alloy_rpc_types::BlockTransactions;
use alloy_rpc_types::TransactionTrait;
use reth_rpc_eth_types::{EthApiError, EthResult, GasPriceOracleConfig, GasPriceOracleResult};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_evm::Evm;
use sov_modules_api::ApiStateAccessor;
use tokio::sync::Mutex;
use tracing::warn;

use super::cache::BlockCache;

/// The number of transactions sampled in a block
pub const SAMPLE_NUMBER: u32 = 3;

/// Calculates a gas price depending on recent blocks.
/// TODO: replace with [`reth_rpc_eth_types::GasPriceOracle`].
pub struct GasPriceOracle<S: sov_modules_api::Spec> {
    /// The type used to get block and tx info
    provider: Evm<S>,
    /// The config for the oracle
    oracle_config: GasPriceOracleConfig,
    /// The latest calculated price and its block hash
    last_price: Mutex<GasPriceOracleResult>,
    /// Cache
    cache: BlockCache<S>,
}

impl<S: sov_modules_api::Spec> GasPriceOracle<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Creates and returns the [`GasPriceOracle`].
    pub fn new(provider: Evm<S>, mut oracle_config: GasPriceOracleConfig) -> Self {
        // sanitize the percentile to be less than 100
        if oracle_config.percentile > 100 {
            warn!(prev_percentile = ?oracle_config.percentile, "Invalid configured gas price percentile, assuming 100");
            oracle_config.percentile = 100;
        }

        let max_header_history = oracle_config.max_header_history;

        Self {
            provider: provider.clone(),
            oracle_config,
            last_price: Default::default(),
            cache: BlockCache::<S>::new(max_header_history as u32, provider),
        }
    }

    /// Suggests a gas price estimate based on recent blocks, using the configured percentile.
    pub async fn suggest_tip_cap(&self, state: &mut ApiStateAccessor<S>) -> EthResult<U256> {
        let header = &self
            .provider
            .get_block_by_number(None, None, state)
            .unwrap()
            .unwrap()
            .header;

        let mut last_price = self.last_price.lock().await;

        // if we have stored a last price, then we check whether or not it was for the same head
        if last_price.block_hash == header.hash {
            return Ok(last_price.price);
        }

        // if all responses are empty, then we can return a maximum of 2*check_block blocks' worth
        // of prices
        //
        // we only return more than check_block blocks' worth of prices if one or more return empty
        // transactions
        let mut current_hash = header.hash;
        let mut results = Vec::new();
        let mut populated_blocks = 0;

        // we only check a maximum of 2 * max_block_history, or the number of blocks in the chain
        let max_blocks = if self.oracle_config.max_block_history * 2 > header.number {
            header.number
        } else {
            self.oracle_config.max_block_history * 2
        };

        for _ in 0..max_blocks {
            let (parent_hash, block_values) = self
                .get_block_values(current_hash, SAMPLE_NUMBER as usize, state)
                .await?
                .ok_or(EthApiError::UnknownBlockOrTxIndex)?;

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
            block_hash: header.hash,
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
        block_hash: B256,
        limit: usize,
        state: &mut ApiStateAccessor<S>,
    ) -> EthResult<Option<(B256, Vec<U256>)>> {
        // check the cache (this will hit the disk if the block is not cached)
        let block = match self.cache.get_block(block_hash, state)? {
            Some(block) => block,
            None => return Ok(None),
        };

        // sort the transactions by effective tip
        // but first filter those that should be ignored

        // get the transactions (block.transactions is a enum but we only care about the 2nd arm)
        let txs = match block.transactions {
            BlockTransactions::Full(txs) => txs,
            _ => return Ok(None),
        };

        let mut effective_gas_prices = txs
            .into_iter()
            .filter(|tx| {
                if let Some(ignore_under) = self.oracle_config.ignore_price {
                    let effective_gas_tip = tx.effective_gas_price(block.header.base_fee_per_gas);
                    if U256::from(effective_gas_tip) < ignore_under {
                        return false;
                    }
                }
                // check if coinbase
                let sender = tx.inner.signer();
                sender != block.header.beneficiary()
            })
            .map(|tx| U256::from(tx.effective_gas_price(block.header.base_fee_per_gas)))
            .collect::<Vec<U256>>();

        effective_gas_prices.sort_unstable();

        effective_gas_prices.truncate(limit);

        Ok(Some((block.header.parent_hash, effective_gas_prices)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Takes only 8 least significant bytes
    fn convert_u256_to_u64(u256: U256) -> u64 {
        u256.wrapping_to()
    }

    #[test_strategy::proptest]
    fn converts_back_and_forth(input: u64) {
        let mut bytes: [u8; 32] = [0; 32];
        for (i, b) in input.to_be_bytes().into_iter().enumerate() {
            let idx = 24 + i;
            bytes[idx] = b;
        }

        let u256 = U256::from_be_slice(&bytes);
        let output = convert_u256_to_u64(u256);

        assert_eq!(input, output);
    }

    #[test_strategy::proptest]
    fn convert_u256_to_u64_doesnt_panic(input: [u8; 32]) {
        let u256 = U256::from_be_slice(&input);
        let _output = convert_u256_to_u64(u256);
    }
}
