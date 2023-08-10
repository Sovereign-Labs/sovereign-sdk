use core::borrow::Borrow;
use core::marker::PhantomData;

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

impl<K, V> StateMap<K, V>
where
    K: BorshSerialize,
    V: BorshSerialize + BorshDeserialize,
{
    /// Creates a new `StateMap`.
    pub const fn new(prefix: Prefix) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            prefix,
        }
    }

    /// Inserts a key-value pair into the map.
    pub fn set<S, Q>(&self, key: &Q, value: &V, working_set: &mut WorkingSet<S>)
    where
        S: Storage,
        Q: BorshSerialize + ?Sized,
        K: Borrow<Q>,
    {
        working_set.set_value(self.prefix(), &key, value)
    }

    /// Returns the value corresponding to the key or None if key is absent in the StateMap.
    ///
    /// # Examples
    ///
    /// We can use as argument any type that can be borrowed by the key.
    ///
    /// ```rust
    ///
    /// fn foo<S>(map: StateMap<Vec<u8>, u64>, key: &[u8], ws: &mut WorkingSet<S>) -> Option<u64>
    /// where
    ///     S: Storage,
    /// {
    ///     // we perform the `get` with a slice, and not the `Vec`. it is so because `Vec` borrows
    ///     // `[T]`.
    ///     map.get(&key[..], ws)
    /// }
    /// ```
    ///
    /// However, some concrete types won't implement `Borrow`, but we can easily cast them into
    /// common types that will
    ///
    /// ```rust
    ///
    /// fn foo<S>(map: StateMap<Vec<u8>, u64>, key: [u8; 32], ws: &mut WorkingSet<S>) -> Option<u64>
    /// where
    ///     S: Storage,
    /// {
    ///     map.get(&key[..], ws)
    /// }
    /// ```
    pub fn get<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Option<V>
    where
        S: Storage,
        Q: BorshSerialize + ?Sized,
        K: Borrow<Q>,
    {
        working_set.get_value(self.prefix(), &key)
    }

    /// Returns the value corresponding to the key or Error if key is absent in the StateMap.
    ///
    /// For reference, check [Self::get].
    pub fn get_or_err<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Result<V, Error>
    where
        S: Storage,
        Q: BorshSerialize + ?Sized,
        K: Borrow<Q>,
    {
        self.get(key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), &key))
        })
    }

    /// Removes a key from the StateMap, returning the corresponding value (or None if the key is absent).
    pub fn remove<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Option<V>
    where
        S: Storage,
        Q: BorshSerialize + ?Sized,
        K: Borrow<Q>,
    {
        working_set.remove_value(self.prefix(), &key)
    }

    /// Removes a key from the StateMap, returning the corresponding value (or Error if the key is absent).
    pub fn remove_or_err<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Result<V, Error>
    where
        S: Storage,
        Q: BorshSerialize + ?Sized,
        K: Borrow<Q>,
    {
        self.remove(&key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), &key))
        })
    }

    /// Deletes a key from the StateMap.
    pub fn delete<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>)
    where
        S: Storage,
        Q: BorshSerialize + ?Sized,
        K: Borrow<Q>,
    {
        working_set.delete_value(self.prefix(), &key);
    }

    /// Returns the storage prefix for the instance.
    pub const fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}
