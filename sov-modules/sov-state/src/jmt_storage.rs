use crate::storage::{Storage, StorageKey, StorageValue};
use first_read_last_write_cache::cache::CacheLog;
use jellyfish_merkle_generic::Version;

// Storage backed by JMT.
pub struct JmtStorage {
    // Caches first read and last write for a particular key.
    _cache: CacheLog,
    _version: Version,
}

impl Storage for JmtStorage {
    fn get(&self, _key: StorageKey) -> Option<StorageValue> {
        todo!()
    }

    fn set(&mut self, _key: StorageKey, _value: StorageValue) {
        todo!()
    }

    fn delete(&mut self, _key: StorageKey) {
        todo!()
    }
}
