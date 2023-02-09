use crate::{backend::Backend, storage::StorageKey, Prefix, Storage};
use sovereign_sdk::serial::{Decode, Encode};

/// A container that maps keys to values.
#[derive(Debug)]
pub struct StateMap<K, V, S> {
    backend: Backend<K, V, S>,
}

impl<K: Encode, V: Encode + Decode, S: Storage> StateMap<K, V, S> {
    pub fn new(storage: S, prefix: Prefix) -> Self {
        Self {
            backend: Backend::new(storage, prefix),
        }
    }

    /// Inserts a key-value pair into the map.
    pub fn set(&mut self, key: K, value: V) {
        let storage_key = StorageKey::new(self.backend.prefix(), key);
        self.backend.set_value(storage_key, value)
    }

    /// Returns the value corresponding to the key or None if key is absent in the StateMap.
    pub fn get(&self, key: K) -> Option<V> {
        let storage_key = StorageKey::new(self.backend.prefix(), key);
        self.backend.get_value(storage_key)
    }

    pub fn prefix(&self) -> &Prefix {
        self.backend.prefix()
    }
}
