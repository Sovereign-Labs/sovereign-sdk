use crate::{
    internal_cache::{StorageInternalCache, ValueReader},
    storage::{GenericStorage, StorageKey, StorageValue},
};
use first_read_last_write_cache::cache::{CacheLog, FirstReads};
use jellyfish_merkle_generic::Version;

#[derive(Default, Clone)]
pub struct JmtDb {
    _version: Version,
}

impl ValueReader for JmtDb {
    fn read_value(&self, _key: StorageKey) -> Option<StorageValue> {
        None
    }
}

/// Storage backed by JmtDb.
pub type JmtStorage = GenericStorage<JmtDb>;

impl JmtStorage {
    /// Creates a new JmtStorage.
    pub fn new(jmt: JmtDb) -> Self {
        Self {
            internal_cache: StorageInternalCache::default(),
            value_reader: jmt,
        }
    }

    /// Gets the first reads from the JmtStorage.
    pub fn get_first_reads(&self) -> FirstReads {
        let cache: &CacheLog = &self.internal_cache.cache.borrow();
        cache.get_first_reads()
    }
}
