use crate::{backend::Backend, storage::StorageKey, Prefix, Storage, WorkingSet};
use sovereign_sdk::serial::{Decode, Encode};
use thiserror::Error;

/// A container that maps keys to values.
#[derive(Debug)]
pub struct StateMap<K, V, S: Storage> {
    backend: Backend<K, V, S>,
}

/// Error type for `StateMap` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0} and: storage key {1}")]
    MissingValue(Prefix, StorageKey),
}

impl<K: Encode, V: Encode + Decode, S: Storage> StateMap<K, V, S> {
    pub fn new(storage: WorkingSet<S>, prefix: Prefix) -> Self {
        Self {
            backend: Backend::new(storage, prefix),
        }
    }

    /// Inserts a key-value pair into the map.
    pub fn set(&mut self, key: &K, value: V, working_set: &mut WorkingSet<S>) {
        let storage_key = StorageKey::new(self.prefix(), key);
        self.backend.set_value(storage_key, value, working_set)
    }

    /// Returns the value corresponding to the key or None if key is absent in the StateMap.
    pub fn get(&self, key: &K, working_set: &mut WorkingSet<S>) -> Option<V> {
        let storage_key = StorageKey::new(self.prefix(), key);
        self.backend.get_value(storage_key, working_set)
    }

    /// Returns the value corresponding to the key or Error if key is absent in the StateMap.
    pub fn get_or_err(&self, key: &K, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.get(key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), key))
        })
    }

    // Removes a key from the StateMap, returning the corresponding value (or None if the key is absent).
    pub fn remove(&mut self, key: &K, working_set: &mut WorkingSet<S>) -> Option<V> {
        let storage_key = StorageKey::new(self.prefix(), key);
        self.backend.remove_value(storage_key, working_set)
    }

    // Removes a key from the StateMap, returning the corresponding value (or Error if the key is absent).
    pub fn remove_or_err(&mut self, key: &K, working_set: &mut WorkingSet<S>) -> Result<V, Error> {
        self.remove(key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), key))
        })
    }

    pub fn prefix(&self) -> &Prefix {
        self.backend.prefix()
    }
}
