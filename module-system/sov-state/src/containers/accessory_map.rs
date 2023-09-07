use std::marker::PhantomData;

use super::StateMapError;
use crate::codec::{BorshCodec, EncodeKeyLike, StateCodec, StateKeyCodec, StateValueCodec};
use crate::storage::StorageKey;
use crate::{AccessoryWorkingSet, Prefix, StateReaderAndWriter, Storage};

/// A container that maps keys to values stored as "accessory" state, outside of
/// the JMT.
///
/// # Type parameters
/// [`AccessoryStateMap`] is generic over:
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
pub struct AccessoryStateMap<K, V, Codec = BorshCodec> {
    _phantom: (PhantomData<K>, PhantomData<V>),
    codec: Codec,
    prefix: Prefix,
}

impl<K, V> AccessoryStateMap<K, V> {
    /// Creates a new [`AccessoryStateMap`] with the given prefix and the default
    /// [`StateValueCodec`] (i.e. [`BorshCodec`]).
    pub fn new(prefix: Prefix) -> Self {
        Self::with_codec(prefix, BorshCodec)
    }
}

impl<K, V, Codec> AccessoryStateMap<K, V, Codec> {
    /// Creates a new [`AccessoryStateMap`] with the given prefix and [`StateValueCodec`].
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            _phantom: (PhantomData, PhantomData),
            codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`AccessoryStateMap`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }
}

impl<K, V, Codec> AccessoryStateMap<K, V, Codec>
where
    Codec: StateCodec,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
{
    /// Inserts a key-value pair into the map.
    ///
    /// Much like [`AccessoryStateMap::get`], the key may be any borrowed form of the
    /// map’s key type.
    pub fn set<Q, S: Storage>(&self, key: &Q, value: &V, working_set: &mut AccessoryWorkingSet<S>)
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
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
    /// use sov_state::{AccessoryStateMap, Storage, AccessoryWorkingSet};
    ///
    /// fn foo<S>(map: AccessoryStateMap<Vec<u8>, u64>, key: &[u8], ws: &mut AccessoryWorkingSet<S>) -> Option<u64>
    /// where
    ///     S: Storage,
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
    /// use sov_state::{AccessoryStateMap, Storage, AccessoryWorkingSet};
    ///
    /// fn foo<S>(map: AccessoryStateMap<Vec<u8>, u64>, key: [u8; 32], ws: &mut AccessoryWorkingSet<S>) -> Option<u64>
    /// where
    ///     S: Storage,
    /// {
    ///     map.get(&key[..], ws)
    /// }
    /// ```
    pub fn get<Q, S: Storage>(&self, key: &Q, working_set: &mut AccessoryWorkingSet<S>) -> Option<V>
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        working_set.get_value(self.prefix(), key, &self.codec)
    }

    /// Returns the value corresponding to the key or [`StateMapError`] if key is absent in
    /// the map.
    pub fn get_or_err<Q, S: Storage>(
        &self,
        key: &Q,
        working_set: &mut AccessoryWorkingSet<S>,
    ) -> Result<V, StateMapError>
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
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
    pub fn remove<Q, S: Storage>(
        &self,
        key: &Q,
        working_set: &mut AccessoryWorkingSet<S>,
    ) -> Option<V>
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        working_set.remove_value(self.prefix(), key, &self.codec)
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`StateMapError`] if the key is absent).
    ///
    /// Use [`AccessoryStateMap::remove`] if you want an [`Option`] instead of a [`Result`].
    pub fn remove_or_err<Q, S: Storage>(
        &self,
        key: &Q,
        working_set: &mut AccessoryWorkingSet<S>,
    ) -> Result<V, StateMapError>
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
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
    /// This is equivalent to [`AccessoryStateMap::remove`], but doesn't deserialize and
    /// return the value beforing deletion.
    pub fn delete<Q, S: Storage>(&self, key: &Q, working_set: &mut AccessoryWorkingSet<S>)
    where
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Q: ?Sized,
    {
        working_set.delete_value(self.prefix(), key, &self.codec);
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, K, V, Codec> AccessoryStateMap<K, V, Codec>
where
    K: arbitrary::Arbitrary<'a>,
    V: arbitrary::Arbitrary<'a>,
    Codec: StateCodec + Default,
    Codec::KeyCodec: StateKeyCodec<K>,
    Codec::ValueCodec: StateValueCodec<V>,
{
    pub fn arbitrary_workset<S>(
        u: &mut arbitrary::Unstructured<'a>,
        working_set: &mut AccessoryWorkingSet<S>,
    ) -> arbitrary::Result<Self>
    where
        S: Storage,
    {
        use arbitrary::Arbitrary;

        let prefix = Prefix::arbitrary(u)?;
        let len = u.arbitrary_len::<(K, V)>()?;
        let codec = Codec::default();
        let map = Self::with_codec(prefix, codec);

        (0..len).try_fold(map, |map, _| {
            let key = K::arbitrary(u)?;
            let value = V::arbitrary(u)?;

            map.set(&key, &value, working_set);

            Ok(map)
        })
    }
}
