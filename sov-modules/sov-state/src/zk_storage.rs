use first_read_last_write_cache::cache::{self, FirstReads};

use crate::{
    internal_cache::{Cache, GetValue},
    storage::{StorageKey, StorageValue},
    Storage,
};

impl GetValue for FirstReads {
    fn get_value(&self, key: StorageKey) -> Option<StorageValue> {
        let key = key.as_cache_key();
        match self.read(&key) {
            cache::ExistsInCache::Yes(read) => read.value.map(|v| StorageValue { value: v }),
            cache::ExistsInCache::No => panic!("todo"),
        }
    }
}

#[derive(Default, Clone)]
pub struct ZkStorage {
    // Caches first read and last write for a particular key.
    cache: Cache,
    first_reads: FirstReads,
}

impl ZkStorage {
    pub fn new(first_reads: FirstReads) -> Self {
        Self {
            cache: Cache::default(),
            first_reads,
        }
    }
}

impl Storage for ZkStorage {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.cache.get(key, &self.first_reads)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.cache.delete(key)
    }
}
