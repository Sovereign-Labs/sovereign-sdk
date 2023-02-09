use crate::{
    storage::{StorageKey, StorageValue},
    Prefix, Storage,
};
use sovereign_sdk::serial::{Decode, Encode};
use std::marker::PhantomData;

/// A container that maps keys to values.
#[derive(Debug)]
pub struct StateMap<K, V, S> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    storage: S,
    // Every instance of the `StateMap` contains a unique prefix.
    // The prefix is prepended to each key before insertion and retrieval from the storage.
    prefix: Prefix,
}

impl<K: Encode, V: Encode + Decode, S: Storage> StateMap<K, V, S> {
    pub fn new(storage: S, prefix: Prefix) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            storage,
            prefix,
        }
    }

    /// Inserts a key-value pair into the map.
    pub fn set(&mut self, key: K, value: V) {
        let storage_key = StorageKey::new(&self.prefix, key);
        self.set_value(storage_key, value)
    }

    /// Returns the value corresponding to the key or None if key is absent in the StateMap.
    pub fn get(&self, key: K) -> Option<V> {
        let storage_key = StorageKey::new(&self.prefix, key);
        self.get_value(storage_key)
    }

    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    pub(crate) fn set_value(&mut self, storage_key: StorageKey, value: V) {
        let storage_value = StorageValue::new(value);
        self.storage.set(storage_key, storage_value);
    }

    pub(crate) fn get_value(&self, storage_key: StorageKey) -> Option<V> {
        let storage_value = self.storage.get(storage_key)?.value;

        let mut storage_reader: &[u8] = &storage_value;
        // It is ok to panic here. Deserialization problem means that something is terribly wrong.
        Some(
            V::decode(&mut storage_reader)
                .unwrap_or_else(|e| panic!("Unable to deserialize storage value {e:?}")),
        )
    }
}
