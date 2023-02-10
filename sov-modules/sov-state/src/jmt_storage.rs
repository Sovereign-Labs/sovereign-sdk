use crate::{
    internal_cache::{Cache, GetValue},
    storage::{Storage, StorageKey, StorageValue},
};
use jellyfish_merkle_generic::Version;

#[derive(Default, Clone)]
struct JMT {}

impl GetValue for JMT {
    fn get_value(&self, key: StorageKey) -> Option<StorageValue> {
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
