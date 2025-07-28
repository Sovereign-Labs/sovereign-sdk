use std::sync::Mutex;

use reth_primitives::B256;
use reth_rpc_eth_types::EthResult;
use reth_rpc_types::{Block, Rich};
use schnellru::{ByLength, LruMap};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::ApiStateAccessor;
/// Block cache for gas oracle
pub struct BlockCache<S: sov_modules_api::Spec> {
    cache: Mutex<LruMap<B256, Rich<Block>, ByLength>>,
    provider: sov_evm::Evm<S>,
}

impl<S: sov_modules_api::Spec> BlockCache<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    pub fn new(max_size: u32, provider: sov_evm::Evm<S>) -> Self {
        Self {
            cache: Mutex::new(LruMap::new(ByLength::new(max_size))),
            provider,
        }
    }

    /// Gets block from cache or from provider
    pub fn get_block(
        &self,
        block_hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> EthResult<Option<Rich<Block>>> {
        // Check if block is in cache
        let mut cache = self.cache.lock().unwrap();
        if let Some(block) = cache.get(&block_hash) {
            return Ok(Some(block.clone()));
        }

        // Get block from provider
        let block = self
            .provider
            .get_block_by_hash(block_hash, Some(true), state)
            .unwrap_or(None);

        // Add a block to cache if it exists
        if let Some(block) = &block {
            cache.insert(block_hash, block.clone());
        }

        Ok(block)
    }
}
