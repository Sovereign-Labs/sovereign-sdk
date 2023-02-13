use first_read_last_write_cache::cache::{self, FirstReads};

use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{StorageKey, StorageValue},
    Storage,
};

// Implementation of `ValueReader` trait for the zk-context. FirstReads is backed by a HashMap internally,
// this is a good default choice. Once we start integrating with a proving system
// we might want to explore other alternatives. For example, in Risc0 we could implement `ValueReader`
// in terms of `env::read()` and fetch values lazily from the host.
impl ValueReader for FirstReads {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        let key = key.as_cache_key();
        match self.get(&key) {
            cache::ValueExists::Yes(read) => read.value.map(|v| StorageValue { value: v }),
            // It is ok to panic here, `ZkStorage` must be able to access all the keys it needs.
            cache::ValueExists::No => panic!("Error: Key {key:?} is inaccessible"),
        }
    }
}

/// Storage that can be used in zk-context.
#[derive(Default, Clone)]
pub struct ZkStorage {
    // Caches first read and last write for a particular key.
    internal_cache: StorageInternalCache,
    first_reads: FirstReads,
}

impl ZkStorage {
    pub fn new(first_reads: FirstReads) -> Self {
        Self {
            internal_cache: StorageInternalCache::default(),
            first_reads,
        }
    }
}

impl Storage for ZkStorage {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.internal_cache.get_or_fetch(key, &self.first_reads)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.internal_cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.internal_cache.delete(key)
    }
}
