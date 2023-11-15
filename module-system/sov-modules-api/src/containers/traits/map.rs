use sov_modules_core::{
    EncodeKeyLike, Prefix, StateCodec, StateKeyCodec, StateReaderAndWriter, StateValueCodec,
    StorageKey,
};
use thiserror::Error;

/// Error type for the get method of state maps.
#[derive(Debug, Error)]
pub enum StateMapError {
    /// Value not found.
    #[error("Value not found for prefix: {0} and: storage key {1}")]
    MissingValue(Prefix, StorageKey),
}

/// Allows a type to access a map from keys to values in state.
pub trait StateMapAccessor<K, V, Codec, W>
where
    Codec: StateCodec,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
    W: StateReaderAndWriter,
{
    /// Returns a reference to the codec used to encode this map.
    fn codec(&self) -> &Codec;

    /// Returns the prefix used when this map was created.
    fn prefix(&self) -> &Prefix;

    /// Inserts a key-value pair into the map.
    ///
    /// The key may be any borrowed form of the
    /// map’s key type.
    fn set<Q>(&self, key: &Q, value: &V, working_set: &mut W)
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        working_set.set_value(self.prefix(), key, value, self.codec())
    }

    /// Returns the value corresponding to the key, or [`None`] if the map
    /// doesn't contain the key.
    ///
    /// # Examples
    ///
    /// The key may be any item that implements [`EncodeKeyLike`] the map's key type
    /// using your chosen codec.
    ///
    /// ```
    /// use sov_modules_api::{StateMapAccessor, Context, StateMap, WorkingSet};
    ///
    /// fn foo<C: Context>(map: StateMap<Vec<u8>, u64>, key: &[u8], ws: &mut WorkingSet<C>) -> Option<u64>
    /// {
    ///     // We perform the `get` with a slice, and not the `Vec`. it is so because `Vec` borrows
    ///     // `[T]`.
    ///     map.get(key, ws)
    /// }
    /// ```
    ///
    /// If the map's key type does not implement [`EncodeKeyLike`] for your desired
    /// target type, you'll have to convert the key to something else. An
    /// example of this would be "slicing" an array to use in [`Vec`]-keyed
    /// maps:
    ///
    /// ```
    /// use sov_modules_api::{StateMapAccessor, Context, StateMap, WorkingSet};
    ///
    /// fn foo<C: Context>(map: StateMap<Vec<u8>, u64>, key: [u8; 32], ws: &mut WorkingSet<C>) -> Option<u64>
    /// {
    ///     map.get(&key[..], ws)
    /// }
    /// ```
    fn get<Q>(&self, key: &Q, working_set: &mut W) -> Option<V>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        working_set.get_value(self.prefix(), key, self.codec())
    }

    /// Returns the value corresponding to the key or [`StateMapError`] if key is absent from
    /// the map.
    fn get_or_err<Q>(&self, key: &Q, working_set: &mut W) -> Result<V, StateMapError>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        self.get(key, working_set).ok_or_else(|| {
            StateMapError::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, self.codec().key_codec()),
            )
        })
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`None`] if the key is absent).
    fn remove<Q>(&self, key: &Q, working_set: &mut W) -> Option<V>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        working_set.remove_value(self.prefix(), key, self.codec())
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`StateMapError`] if the key is absent).
    ///
    /// Use [`StateMapAccessor::remove`] if you want an [`Option`] instead of a [`Result`].
    fn remove_or_err<Q>(&self, key: &Q, working_set: &mut W) -> Result<V, StateMapError>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
    {
        self.remove(key, working_set).ok_or_else(|| {
            StateMapError::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, self.codec().key_codec()),
            )
        })
    }

    /// Deletes a key-value pair from the map.
    ///
    /// This is equivalent to [`StateMapAccessor::remove`], but doesn't deserialize and
    /// return the value before deletion.
    fn delete<Q>(&self, key: &Q, working_set: &mut W)
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        working_set.delete_value(self.prefix(), key, self.codec());
    }
}
