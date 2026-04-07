use std::any::Any;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use alloy_primitives::B256;
use sov_modules_api::{ApiStateAccessor, Spec};

use crate::primitive_types::SyntheticBlockWithoutRootsAndBloom;

// Prune synthetic blocks more than this number of blocks away from the latest block.
const SYNTHETIC_BLOCKS_CACHE_PRUNE_INTERVAL: u64 = 20;

/// Cache of synthetic block states by hash.
///
/// Currently, there's no way to recreate the state of a synthetic block once it's gone. This is because
/// EVM state is shared with the sov-modules system, so any sov txs can impact the state of the synthetic block - but only
/// EVM tx bodies are stored in the EVM module. This cache saves a copy of the API state accessor for each synthetic block we make,
/// pruning them after some interval.
pub(crate) static SAVED_SYNTHETIC_BLOCK_STATE_BY_HASH: OnceLock<RwLock<SyntheticBlocksCache>> =
    OnceLock::new();

#[derive(Default)]
pub(crate) struct SyntheticBlocksCache {
    // Store ApiStateAccessor<S> by hash. We have to type erase because static variables can't store generic types.
    state_by_hash: HashMap<B256, Box<dyn Any + Send + Sync>>,
    // To allow for pruning, save a list of all synthetic block hashes for each block number
    // When those blocks are old enough, we prune them all.
    hashes_by_block_number: HashMap<u64, Vec<B256>>,
    oldest_block_number: u64,
}
impl SyntheticBlocksCache {
    pub(crate) fn insert_if_absent<S: Spec>(
        &mut self,
        block: &SyntheticBlockWithoutRootsAndBloom,
        state: &mut ApiStateAccessor<S>,
    ) {
        if let Entry::Vacant(entry) = self.state_by_hash.entry(block.hash()) {
            let state = state.clone_without_local_writes();
            let state: Box<dyn Any + Send + Sync> = Box::new(state);
            entry.insert(state);
            self.hashes_by_block_number
                .entry(block.block_number())
                .or_default()
                .push(block.hash());
            if block.block_number()
                > self.oldest_block_number + SYNTHETIC_BLOCKS_CACHE_PRUNE_INTERVAL
            {
                self.prune(block.block_number());
            }
        }
    }

    pub(crate) fn get_state_by_hash<S: Spec>(&self, hash: B256) -> Option<&ApiStateAccessor<S>> {
        self.state_by_hash.get(&hash).map(|b| {
            b.downcast_ref::<ApiStateAccessor<S>>()
                .expect("SyntheticBlocksCache: spec type mismatch (bug)")
        })
    }
    fn prune(&mut self, block_number: u64) {
        let stop_at = block_number.saturating_sub(SYNTHETIC_BLOCKS_CACHE_PRUNE_INTERVAL);
        for number in self.oldest_block_number..stop_at {
            let hashes = self
                .hashes_by_block_number
                .remove(&number)
                .unwrap_or_default();
            for hash in hashes {
                self.state_by_hash.remove(&hash);
            }
        }
        self.oldest_block_number = stop_at;
    }
}
