use crate::{
    internal_cache::{Cache, ValueReader},
    storage::{Storage, StorageKey, StorageValue},
};
use first_read_last_write_cache::cache::{CacheLog, FirstReads};
use jellyfish_merkle_generic::Version;

#[derive(Default, Clone)]
struct JMT {}

impl ValueReader for JMT {
    fn read_value(&self, key: StorageKey) -> Option<StorageValue> {
        todo!()
    }
}

// Storage backed by JMT.
#[derive(Default, Clone)]
pub struct JmtStorage {
    // Caches first read and last write for a particular key.
    cache: Cache,
    jmt: JMT,
    _version: Version,
}

impl JmtStorage {
    pub fn reads(&self) -> FirstReads {
        let cache: &CacheLog = &self.cache.cache.borrow();
        cache.get_first_reads()
    }
}

impl Storage for JmtStorage {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.cache.get(key, &self.jmt)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.cache.delete(key)
    }
}
