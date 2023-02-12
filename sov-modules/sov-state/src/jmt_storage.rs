use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{Storage, StorageKey, StorageValue},
};
use first_read_last_write_cache::cache::{CacheLog, FirstReads};
use jellyfish_merkle_generic::Version;

#[derive(Default, Clone)]
pub struct JmtDb {}

impl ValueReader for JmtDb {
    fn read_value(&self, _key: StorageKey) -> Option<StorageValue> {
        todo!()
    }
}

/// Storage backed by JmtDb.
#[derive(Default, Clone)]
pub struct JmtStorage {
    // Caches first read and last write for a particular key.
    internal_cache: StorageInternalCache,
    jmt: JmtDb,
    _version: Version,
}

impl JmtStorage {
    ///
    pub fn new(jmt: JmtDb, _version: Version) -> Self {
        Self {
            internal_cache: StorageInternalCache::default(),
            jmt,
            _version,
        }
    }

    ///
    pub fn reads(&self) -> FirstReads {
        let cache: &CacheLog = &self.internal_cache.cache.borrow();
        cache.get_first_reads()
    }
}

impl Storage for JmtStorage {
    fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.internal_cache.get_or_fetch(key, &self.jmt)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.internal_cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.internal_cache.delete(key)
    }
}
