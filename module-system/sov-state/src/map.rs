use core::marker::PhantomData;
use std::borrow::Borrow;

use thiserror::Error;

use crate::codec::{BorshCodec, StateCodec, StateKeyEncodePreservingBorrow};
use crate::storage::StorageKey;
use crate::{Prefix, Storage, WorkingSet};

/// A container that maps keys to values.
///
/// # Type parameters
/// [`StateMap`] is generic over:
/// - a key type (`K`);
/// - a value type (`V`);
/// - a [`StateCodec`] (`C`).
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct StateMap<K, V, C = BorshCodec>
where
    C: StateCodec<K, V>,
{
    _phantom: (PhantomData<K>, PhantomData<V>),
    codec: C,
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
    BorshCodec: StateCodec<K, V>,
{
    /// Creates a new [`StateMap`] with the given prefix and the default
    /// [`StateCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            codec: BorshCodec,
            prefix,
        }
    }
}

impl<K, V, C> StateMap<K, V, C>
where
    C: StateCodec<K, V>,
{
    /// Creates a new [`StateMap`] with the given prefix and codec.
    ///
    /// Note that `codec` must implement both [`StateKeyCodec`] and
    /// [`StateValueCodec`] and there's no way (yet?) to use different codecs
    /// for keys and values.
    pub fn with_codec(prefix: Prefix, codec: C) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`StateValue`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    /// Inserts a key-value pair into the map.
    pub fn set<S, Q>(&self, key: &Q, value: &V, working_set: &mut WorkingSet<S>)
    where
        S: Storage,
        K: Borrow<Q>,
        C: StateKeyEncodePreservingBorrow<K, Q>,
    {
        working_set.set_value(self.prefix(), &self.codec, key, value)
    }

    /// Returns the value corresponding to the key or None if key is absent in the StateMap.
    ///
    /// # Examples
    ///
    /// We can use as argument any type that can be borrowed by the key.
    ///
    /// ```rust
    /// use sov_state::{StateMap, Storage, WorkingSet};
    ///
    /// fn foo<S>(map: StateMap<Vec<u8>, u64>, key: &[u8], ws: &mut WorkingSet<S>) -> Option<u64>
    /// where
    ///     S: Storage,
    /// {
    ///     // we perform the `get` with a slice, and not the `Vec`. it is so because `Vec` borrows
    ///     // `[T]`.
    ///     map.get(key, ws)
    /// }
    /// ```
    ///
    /// However, some concrete types won't implement `Borrow`, but we can easily cast them into
    /// common types that will
    ///
    /// ```rust
    /// use sov_state::{StateMap, Storage, WorkingSet};
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
        K: Borrow<Q>,
        C: StateKeyEncodePreservingBorrow<K, Q>,
    {
        working_set.get_value(self.prefix(), &self.codec, key)
    }

    /// Returns the value corresponding to the key or Error if key is absent in the StateMap.
    ///
    /// For reference, check [Self::get].
    pub fn get_or_err<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Result<V, Error>
    where
        S: Storage,
        K: Borrow<Q>,
        C: StateKeyEncodePreservingBorrow<K, Q>,
    {
        self.get(key, working_set).ok_or_else(|| {
            Error::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, &self.codec),
            )
        })
    }

    /// Removes a key from the StateMap, returning the corresponding value (or None if the key is absent).
    pub fn remove<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Option<V>
    where
        S: Storage,
        K: Borrow<Q>,
        C: StateKeyEncodePreservingBorrow<K, Q>,
    {
        working_set.remove_value(self.prefix(), &self.codec, key)
    }

    /// Removes a key from the StateMap, returning the corresponding value (or Error if the key is absent).
    pub fn remove_or_err<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>) -> Result<V, Error>
    where
        S: Storage,
        K: Borrow<Q>,
        C: StateKeyEncodePreservingBorrow<K, Q>,
    {
        self.remove(key, working_set).ok_or_else(|| {
            Error::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, &self.codec),
            )
        })
    }

    /// Deletes a key from the StateMap.
    pub fn delete<S, Q>(&self, key: &Q, working_set: &mut WorkingSet<S>)
    where
        S: Storage,
        K: Borrow<Q>,
        C: StateKeyEncodePreservingBorrow<K, Q>,
    {
        working_set.delete_value(self.prefix(), &self.codec, key);
    }
}
