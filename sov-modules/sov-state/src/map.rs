use crate::{storage::StorageKey, Prefix, Storage};
use sovereign_sdk::serial::{Decode, Encode};
use std::marker::PhantomData;

// A container that maps keys to values.
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

    // Inserts a key-value pair into the map.
    pub fn set(&self, key: K, value: V) {
        let storage_key = StorageKey::new(&self.prefix, key);

        let storage_value = value.into();
        self.storage.set(storage_key, storage_value);
    }

    // Returns the value corresponding to the key or None if key is absent in the StateMap.
    pub fn get(&mut self, key: K) -> Option<V> {
        let storage_key = StorageKey::new(&self.prefix, key);
        let storage_value = self.storage.get(storage_key)?.value;
        let mut storage_reader: &[u8] = &storage_value;

        // TODO panic
        Some(V::decode(&mut storage_reader).unwrap())
    }

    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}
