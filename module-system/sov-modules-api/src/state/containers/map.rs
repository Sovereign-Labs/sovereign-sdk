use std::marker::PhantomData;

use sov_state::codec::{BorshCodec, EncodeKeyLike, StateCodec, StateKeyCodec, StateValueCodec};
use sov_state::storage::StorageKey;
use sov_state::Prefix;
use thiserror::Error;

use crate::state::{StateReaderAndWriter, WorkingSet};
use crate::Context;

/// A container that maps keys to values.
///
/// # Type parameters
/// [`StateMap`] is generic over:
/// - a key type `K`;
/// - a value type `V`;
/// - a [`StateValueCodec`] `Codec`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct StateMap<K, V, Codec = BorshCodec> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    codec: Codec,
    prefix: Prefix,
}

/// Error type for the [`StateMap::get`] method.
#[derive(Debug, Error)]
pub enum StateMapError {
    /// Value not found.
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

impl<K, V, Codec> StateMap<K, V, Codec> {
    /// Creates a new [`StateMap`] with the given prefix and [`StateValueCodec`].
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            codec,
            prefix,
        }
    }

    /// Returns a reference to the codec used by this [`StateMap`].
    pub fn codec(&self) -> &Codec {
        &self.codec
    }

    /// Returns the prefix used when this [`StateMap`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

impl<K, V, Codec> StateMap<K, V, Codec>
where
    Codec: StateCodec,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
{
    /// Inserts a key-value pair into the map.
    ///
    /// Much like [`StateMap::get`], the key may be any borrowed form of the
    /// map’s key type.
    pub fn set<Q, C>(&self, key: &Q, value: &V, working_set: &mut WorkingSet<C>)
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
        C: Context,
    {
        working_set.set_value(self.prefix(), key, value, &self.codec)
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
    /// use sov_modules_api::{Context, StateMap, WorkingSet};
    ///
    /// fn foo<C>(map: StateMap<Vec<u8>, u64>, key: &[u8], ws: &mut WorkingSet<C>) -> Option<u64>
    /// where
    ///     C: Context,
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
    /// use sov_modules_api::{Context, StateMap, WorkingSet};
    ///
    /// fn foo<C>(map: StateMap<Vec<u8>, u64>, key: [u8; 32], ws: &mut WorkingSet<C>) -> Option<u64>
    /// where
    ///     C: Context,
    /// {
    ///     map.get(&key[..], ws)
    /// }
    /// ```
    pub fn get<Q, C>(&self, key: &Q, working_set: &mut WorkingSet<C>) -> Option<V>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
        C: Context,
    {
        working_set.get_value(self.prefix(), key, &self.codec)
    }

    /// Returns the value corresponding to the key or [`StateMapError`] if key is absent in
    /// the map.
    pub fn get_or_err<Q, C>(
        &self,
        key: &Q,
        working_set: &mut WorkingSet<C>,
    ) -> Result<V, StateMapError>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
        C: Context,
    {
        self.get(key, working_set).ok_or_else(|| {
            StateMapError::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, self.codec.key_codec()),
            )
        })
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`None`] if the key is absent).
    pub fn remove<Q, C>(&self, key: &Q, working_set: &mut WorkingSet<C>) -> Option<V>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
        C: Context,
    {
        working_set.remove_value(self.prefix(), key, &self.codec)
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`StateMapError`] if the key is absent).
    ///
    /// Use [`StateMap::remove`] if you want an [`Option`] instead of a [`Result`].
    pub fn remove_or_err<Q, C>(
        &self,
        key: &Q,
        working_set: &mut WorkingSet<C>,
    ) -> Result<V, StateMapError>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
        Q: ?Sized,
        C: Context,
    {
        self.remove(key, working_set).ok_or_else(|| {
            StateMapError::MissingValue(
                self.prefix().clone(),
                StorageKey::new(self.prefix(), key, self.codec.key_codec()),
            )
        })
    }

    /// Deletes a key-value pair from the map.
    ///
    /// This is equivalent to [`StateMap::remove`], but doesn't deserialize and
    /// return the value beforing deletion.
    pub fn delete<Q, C>(&self, key: &Q, working_set: &mut WorkingSet<C>)
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
        C: Context,
    {
        working_set.delete_value(self.prefix(), key, &self.codec);
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, K, V, Codec> StateMap<K, V, Codec>
where
    K: arbitrary::Arbitrary<'a>,
    V: arbitrary::Arbitrary<'a>,
    Codec: StateCodec + Default,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
{
    /// Returns an arbitrary [`StateMap`] instance.
    ///
    /// See the [`arbitrary`] crate for more information.
    pub fn arbitrary_workset<C>(
        u: &mut arbitrary::Unstructured<'a>,
        working_set: &mut WorkingSet<C>,
    ) -> arbitrary::Result<Self>
    where
        C: Context,
    {
        use arbitrary::Arbitrary;

        let prefix = Prefix::arbitrary(u)?;
        let len = u.arbitrary_len::<(K, V)>()?;
        let codec = Codec::default();
        let map = StateMap::with_codec(prefix, codec);

        (0..len).try_fold(map, |map, _| {
            let key = K::arbitrary(u)?;
            let value = V::arbitrary(u)?;

            map.set(&key, &value, working_set);

            Ok(map)
        })
    }
}
