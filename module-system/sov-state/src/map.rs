use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use thiserror::Error;

use crate::storage::StorageKey;
use crate::{Prefix, Storage, WorkingSet};

/// A container that maps keys to values.

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct StateMap<K, V> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    prefix: Prefix,
}

/// Error type for `StateMap` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0} and: storage key {1}")]
    MissingValue(Prefix, StorageKey),
}

impl<K: BorshSerialize, V: BorshSerialize + BorshDeserialize> StateMap<K, V> {
    pub fn new(prefix: Prefix) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            prefix,
        }
    }

    /// Inserts a key-value pair into the map.
    pub fn set<S: Storage>(&self, key: &K, value: &V, working_set: &mut WorkingSet<S>) {
        working_set.set_value(self.prefix(), key, value)
    }

    /// Returns the value corresponding to the key or None if key is absent in the StateMap.
    pub fn get<S: Storage>(&self, key: &K, working_set: &mut WorkingSet<S>) -> Option<V> {
        working_set.get_value(self.prefix(), key)
    }

    /// Returns the value corresponding to the key or Error if key is absent in the StateMap.
    pub fn get_or_err<S: Storage>(
        &self,
        key: &K,
        working_set: &mut WorkingSet<S>,
    ) -> Result<V, Error> {
        self.get(key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), key))
        })
    }

    /// Removes a key from the StateMap, returning the corresponding value (or None if the key is absent).
    pub fn remove<S: Storage>(&self, key: &K, working_set: &mut WorkingSet<S>) -> Option<V> {
        working_set.remove_value(self.prefix(), key)
    }

    /// Removes a key from the StateMap, returning the corresponding value (or Error if the key is absent).
    pub fn remove_or_err<S: Storage>(
        &self,
        key: &K,
        working_set: &mut WorkingSet<S>,
    ) -> Result<V, Error> {
        self.remove(key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), key))
        })
    }

    /// Deletes a key from the StateMap.
    pub fn delete<S: Storage>(&self, key: &K, working_set: &mut WorkingSet<S>) {
        working_set.delete_value(self.prefix(), key);
    }

    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}
