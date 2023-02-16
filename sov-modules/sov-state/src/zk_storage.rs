use first_read_last_write_cache::cache::{self, FirstReads};

use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{GenericStorage, StorageKey, StorageValue},
};

// Implementation of `ValueReader` trait for the zk-context. FirstReads is backed by a HashMap internally,
// this is a good default choice. Once we start integrating with a proving system
// we might want to explore other alternatives. For example, in Risc0 we could implement `ValueReader`
// in terms of `env::read()` and fetch values lazily from the host.
impl ValueReader for FirstReads {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        let key = key.as_cache_key();
        match self.get(&key) {
            cache::ValueExists::Yes(read) => read.map(StorageValue::new_from_cache_value),
            // It is ok to panic here, `ZkStorage` must be able to access all the keys it needs.
            cache::ValueExists::No => panic!("Error: Key {key:?} is inaccessible"),
        }
    }
}

/// Storage that can be used in zk-context.
pub type ZkStorage = GenericStorage<FirstReads>;

impl ZkStorage {
    pub fn new(first_reads: FirstReads) -> Self {
        Self {
            internal_cache: StorageInternalCache::default(),
            value_reader: first_reads,
        }
    }
}
