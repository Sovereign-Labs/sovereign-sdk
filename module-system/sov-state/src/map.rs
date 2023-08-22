use std::borrow::Borrow;
use std::hash::Hash;
use std::marker::PhantomData;

use thiserror::Error;

use crate::codec::{BorshCodec, StateValueCodec};
use crate::storage::StorageKey;
use crate::{Prefix, Storage, WorkingSet};

/// A container that maps keys to values.
///
/// # Type parameters
/// [`StateMap`] is generic over:
/// - a key type `K`;
/// - a value type `V`;
/// - a [`StateValueCodec`] `VC`.
#[derive(Debug, Clone, PartialEq, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct StateMap<K, V, VC = BorshCodec> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    value_codec: VC,
    prefix: Prefix,
}

/// Error type for the [`StateMap::get`] method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0} and: storage key {1}")]
    MissingValue(Prefix, StorageKey),
}

impl<K, V> StateMap<K, V> {
    /// Creates a new [`StateMap`] with the given prefix and the default
    /// [`StateValueCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self::with_codec(prefix, BorshCodec)
    }
}

impl<K, V, VC> StateMap<K, V, VC> {
    /// Creates a new [`StateMap`] with the given prefix and codec.
    pub fn with_codec(prefix: Prefix, codec: VC) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            value_codec: codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`StateMap`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

impl<K, V, VC> StateMap<K, V, VC>
where
    K: Hash + Eq,
    VC: StateValueCodec<V>,
{
    /// Inserts a key-value pair into the map.
    pub fn set<Q, S: Storage>(&self, key: &Q, value: &V, working_set: &mut WorkingSet<S>)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        working_set.set_value(self.prefix(), key, value, &self.value_codec)
    }

    /// Returns the value corresponding to the key or None if key is absent in the StateMap.
    pub fn get<Q, S: Storage>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        working_set.get_value(self.prefix(), key, &self.value_codec)
    }

    /// Returns the value corresponding to the key or Error if key is absent in the StateMap.
    pub fn get_or_err<Q, S: Storage>(
        &self,
        key: &Q,
        working_set: &mut WorkingSet<S>,
    ) -> Result<V, Error>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.get(key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), key))
        })
    }

    /// Removes a key from the StateMap, returning the corresponding value (or None if the key is absent).
    pub fn remove<Q, S: Storage>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        working_set.remove_value(self.prefix(), key, &self.value_codec)
    }

    /// Removes a key from the StateMap, returning the corresponding value (or Error if the key is absent).
    pub fn remove_or_err<Q, S: Storage>(
        &self,
        key: &Q,
        working_set: &mut WorkingSet<S>,
    ) -> Result<V, Error>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.remove(key, working_set).ok_or_else(|| {
            Error::MissingValue(self.prefix().clone(), StorageKey::new(self.prefix(), key))
        })
    }

    /// Deletes a key from the StateMap.
    pub fn delete<Q, S: Storage>(&self, key: &Q, working_set: &mut WorkingSet<S>)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        working_set.delete_value(self.prefix(), key);
    }
}
