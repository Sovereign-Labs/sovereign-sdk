use sov_state::{SlotKeyFromCodec, SlotValueFromCodec};
use std::marker::PhantomData;
use std::str::FromStr;

#[cfg(feature = "native")]
use anyhow::ensure;
use sov_state::codec::BorshCodec;
use sov_state::namespaces::{Accessory, CompileTimeNamespace, Kernel, User};
use sov_state::{EncodeLike, Prefix, SlotKey, SlotValue, StateCodec, StateItemCodec};
#[cfg(feature = "native")]
use sov_state::{NativeStorage, StateItemDecoder, Storage};
use thiserror::Error;
#[cfg(feature = "arbitrary")]
use unwrap_infallible::UnwrapInfallible;

use super::{Borrowed, BorrowedMut};
use crate::state::StateReader;
#[cfg(feature = "native")]
use crate::ProvenStateAccessor;
#[cfg(feature = "arbitrary")]
use crate::{InfallibleStateReaderAndWriter, StateCheckpoint};
use crate::{StateReaderAndWriter, StateWriter};

/// A container that maps keys to values.
///
/// # Type parameters
/// [`StateMap`] is generic over:
/// - a key type `K`;
/// - a value type `V`;
/// - a [`sov_state::StateItemCodec`] `Codec`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct NamespacedStateMap<N, K, V, Codec = BorshCodec>
where
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
    K: FromStr + std::fmt::Display,
{
    _phantom: PhantomData<(N, K, V)>,
    pub(crate) codec: Codec,
    pub(crate) prefix: Prefix,
}

/// Error type for the get method of state maps.
#[derive(Debug, Error)]
pub enum StateMapError<N> {
    /// Value not found.
    #[error("Value not found for prefix: {0} and storage key: {1} in namespace {}", std::any::type_name::<N>())]
    MissingValue(Prefix, SlotKey, PhantomData<N>),
}

type ValueOrError<V, N> = Result<V, StateMapError<N>>;

/// A container that maps keys to values
///
/// # Type parameters
/// [`StateMap`] is generic over:
/// - a key type `K`;
/// - a value type `V`;
/// - a  [`Codec`](`sov_state::StateItemCodec`).
pub type StateMap<K, V, Codec = BorshCodec> = NamespacedStateMap<User, K, V, Codec>;

/// A container that maps keys to values stored as "accessory" state, outside of
/// the JMT.
///
/// # Type parameters
/// [`AccessoryStateMap`] is generic over:
/// - a key type `K`;
/// - a value type `V`;
/// - a  [`Codec`](`sov_state::StateItemCodec`).
pub type AccessoryStateMap<K, V, Codec = BorshCodec> = NamespacedStateMap<Accessory, K, V, Codec>;

/// A container that maps keys to values which are only accessible the kernel.
///
/// # Type parameters
/// [`KernelStateMap`] is generic over:
/// - a key type `K`;
/// - a value type `V`;
/// - a  [`Codec`](`sov_state::StateItemCodec`).
pub type KernelStateMap<K, V, Codec = BorshCodec> = NamespacedStateMap<Kernel, K, V, Codec>;

impl<N, K, V, Codec> NamespacedStateMap<N, K, V, Codec>
where
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
    K: FromStr + std::fmt::Display,
{
    //e Creates a new [`StateMap`] with the given prefix and [`sov_modules_core::StateItemCodec`].
    pub fn with_codec(prefix: Prefix, codec: Codec) -> Self {
        Self {
            _phantom: PhantomData,
            codec,
            prefix,
        }
    }

    /// Returns the prefix used when this [`StateMap`] was created.
    pub fn prefix(&self) -> &Prefix {
        &self.prefix
    }

    /// Returns the codec used when this [`StateMap`] was created.
    pub fn codec(&self) -> &Codec {
        &self.codec
    }
}

impl<N, K, V, Codec> NamespacedStateMap<N, K, V, Codec>
where
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
    K: FromStr + std::fmt::Display,
{
    pub fn slot_key<Kq>(&self, key: &Kq) -> SlotKey
    where
        Kq: ?Sized,
        Codec::KeyCodec: EncodeLike<Kq, K>,
    {
        SlotKey::new(self.prefix(), key, self.codec().key_codec())
    }

    pub(super) fn slot_value<Vq>(&self, value: &Vq) -> SlotValue
    where
        Vq: ?Sized,
        Codec::ValueCodec: EncodeLike<Vq, V>,
    {
        SlotValue::new(value, self.codec().value_codec())
    }

    /// Inserts a key-value pair into the map.
    ///
    /// The key may be any borrowed form of the
    /// map’s key type.
    pub fn set<Kq, Vq, Writer>(
        &mut self,
        key: &Kq,
        value: &Vq,
        state: &mut Writer,
    ) -> Result<(), Writer::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Codec::ValueCodec: EncodeLike<Vq, V>,
        Kq: ?Sized,
        Vq: ?Sized,
        Writer: StateWriter<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Setting map value");
        state.set(&key, self.slot_value(value))
    }

    /// Calls [`StateMap::set`] iff the key is not already present in the map.
    pub fn set_if_absent<Kq, Vq, Writer>(
        &mut self,
        key: &Kq,
        value: &Vq,
        state: &mut Writer,
    ) -> Result<(), <Writer as StateWriter<N>>::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Codec::ValueCodec: EncodeLike<Vq, V>,
        Kq: ?Sized,
        Vq: ?Sized,
        Writer: StateReaderAndWriter<N>,
    {
        if state
            .get_decoded(&self.slot_key(key), self.codec())?
            .is_none()
        {
            self.set(key, value, state)?;
        }

        Ok(())
    }

    /// Returns the value corresponding to the key, or [`None`] if the map
    /// doesn't contain the key.
    ///
    /// # Examples
    ///
    /// The key may be any item that implements [`EncodeLike`] the map's key type
    /// using your chosen codec.
    ///
    /// ```
    /// use sov_modules_api::{Spec, Context, HexString, StateMap, WorkingSet, StateAccessorError};
    ///
    /// fn foo<S: Spec>(map: StateMap<HexString, u64>, key: &[u8], state: &mut WorkingSet<S>) -> Result<Option<u64>, StateAccessorError<S::Gas>>
    /// {
    ///     // We perform the `get` with a slice, and not a owned `HexString`. This works because a `HexString` is just
    ///     // a wrapper around `Vec<u8>` that implements `Display` and `FromStr` - so we can encode any byte slice
    ///     // like a HexString.
    ///     map.get(key, state)
    /// }
    /// ```
    ///
    /// If the map's key type does not implement [`EncodeLike`] for your desired
    /// target type, you'll have to convert the key to something else. An
    /// example of this would be "slicing" an array to use in [`sov_modules_api::HexString`]-keyed
    /// maps:
    ///
    /// ```
    /// use sov_modules_api::{Spec, Context, HexString, StateMap, WorkingSet, StateAccessorError};
    ///
    /// fn foo<S: Spec>(map: StateMap<HexString, u64>, key: [u8; 32], state: &mut WorkingSet<S>) -> Result<Option<u64>, StateAccessorError<S::Gas>>
    /// {
    ///     map.get(key.as_ref(), state)
    /// }
    /// ```
    pub fn get<Kq, Reader>(&self, key: &Kq, state: &mut Reader) -> Result<Option<V>, Reader::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        Reader: StateReader<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Getting map value");
        state.get_decoded(&key, self.codec())
    }

    /// Returns a borrowed value from the map, preventing mutable access to the map until this reference is dropped.
    pub fn borrow<Kq, Reader>(
        &self,
        key: &Kq,
        state: &mut Reader,
    ) -> Result<Borrowed<'_, Option<V>, Self>, Reader::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        Reader: StateReader<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Borrowing map value");
        let val = state.get_decoded(&key, self.codec())?;
        Ok(Borrowed::new(val, self))
    }

    /// Returns a mutably borrowed value from the map, preventing access to the map until this reference is dropped.
    pub fn borrow_mut<Kq, Reader>(
        &mut self,
        key: &Kq,
        state: &mut Reader,
    ) -> Result<BorrowedMut<'_, Option<V>, Self>, Reader::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        Reader: StateReader<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Borrowing map value");
        let val = state.get_decoded(&key, self.codec())?;
        Ok(BorrowedMut::new(key, val, self))
    }

    /// Returns the value corresponding to the key or [`StateMapError`] if key is absent from
    /// the map.
    pub fn get_or_err<Kq, Reader>(
        &self,
        key: &Kq,
        state: &mut Reader,
    ) -> Result<ValueOrError<V, N>, Reader::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        Reader: StateReader<N>,
    {
        Ok(self.get(key, state)?.ok_or_else(|| {
            StateMapError::MissingValue(
                *self.prefix(),
                SlotKey::new(self.prefix(), key, self.codec().key_codec()),
                PhantomData,
            )
        }))
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`None`] if the key is absent).
    pub fn remove<Kq, ReaderAndWriter>(
        &self,
        key: &Kq,
        state: &mut ReaderAndWriter,
    ) -> Result<Option<V>, <ReaderAndWriter as StateWriter<N>>::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        ReaderAndWriter: StateReaderAndWriter<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Removing map value");
        state.remove_decoded(&key, self.codec())
    }

    /// Removes a key from the map, returning the corresponding value (or
    /// [`StateMapError`] if the key is absent).
    ///
    /// Use [`NamespacedStateMap::remove`] if you want an [`Option`] instead of a [`Result`].
    pub fn remove_or_err<Kq, ReaderAndWriter>(
        &mut self,
        key: &Kq,
        state: &mut ReaderAndWriter,
    ) -> Result<ValueOrError<V, N>, <ReaderAndWriter as StateWriter<N>>::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        ReaderAndWriter: StateReaderAndWriter<N>,
    {
        Ok(self.remove(key, state)?.ok_or_else(|| {
            StateMapError::MissingValue(
                *self.prefix(),
                SlotKey::new(self.prefix(), key, self.codec().key_codec()),
                PhantomData,
            )
        }))
    }

    /// Deletes a key-value pair from the map.
    ///
    /// This is equivalent to [`NamespacedStateMap::remove`], but doesn't deserialize and
    /// return the value before deletion, or error on a missing value.
    pub fn delete<Kq, Writer>(&mut self, key: &Kq, state: &mut Writer) -> Result<(), Writer::Error>
    where
        Codec: StateCodec,
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        Writer: StateWriter<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Deleting map value");
        state.delete(&key)
    }
}

#[cfg(feature = "native")]
impl<N, K, V, Codec> NamespacedStateMap<N, K, V, Codec>
where
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
    K: FromStr + std::fmt::Display,
{
    /// Returns the raw value bytes corresponding to the key, or [`None`] if the map doesn't contain the key.
    pub fn get_raw<Kq, Reader>(
        &self,
        key: &Kq,
        state: &mut Reader,
    ) -> Result<Option<Vec<u8>>, Reader::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        Reader: StateReader<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Getting raw map value");
        state.get(&key).map(|value| value.map(|v| v.value().to_vec()))
    }

    /// Inserts a key-value pair into the map where the value is already serialized bytes.
    pub fn set_raw<Kq, Writer>(
        &mut self,
        key: &Kq,
        value: &[u8],
        state: &mut Writer,
    ) -> Result<(), Writer::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        Writer: StateWriter<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Setting raw map value");
        state.set(&key, SlotValue::from(value.to_vec()))
    }

    /// Removes a key from the map, returning the corresponding raw value bytes (or [`None`] if the key is absent).
    pub fn remove_raw<Kq, ReaderAndWriter>(
        &self,
        key: &Kq,
        state: &mut ReaderAndWriter,
    ) -> Result<Option<Vec<u8>>, <ReaderAndWriter as StateWriter<N>>::Error>
    where
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Kq: ?Sized,
        ReaderAndWriter: StateReaderAndWriter<N>,
    {
        let key = self.slot_key(key);
        #[cfg(feature = "expensive-observability")]
        tracing::trace!(%key, "Removing raw map value");
        let value = state.get(&key)?;
        state.delete(&key)?;
        Ok(value.map(|v| v.value().to_vec()))
    }

    /// Iterates raw values for caller-provided keys.
    ///
    /// This is a backend-agnostic fallback when prefix iteration is unavailable.
    pub fn iter_raw_from_keys<I, Kq, Reader>(
        &self,
        keys: I,
        state: &mut Reader,
    ) -> Result<impl Iterator<Item = (Kq, Option<Vec<u8>>)>, Reader::Error>
    where
        I: IntoIterator<Item = Kq>,
        Codec::KeyCodec: EncodeLike<Kq, K>,
        Reader: StateReader<N>,
    {
        let mut entries = Vec::new();
        for key in keys {
            let value = self.get_raw(&key, state)?;
            entries.push((key, value));
        }
        Ok(entries.into_iter())
    }
}

#[cfg(feature = "native")]
impl<N: sov_state::namespaces::ProvableCompileTimeNamespace, K, V, Codec>
    NamespacedStateMap<N, K, V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<K>,
    K: FromStr + std::fmt::Display,
{
    pub fn get_with_proof<Kq, W>(
        &self,
        key: &Kq,
        state: &mut W,
    ) -> Option<sov_state::StorageProof<W::Proof>>
    where
        Kq: ?Sized,
        Codec::KeyCodec: EncodeLike<Kq, K>,
        W: ProvenStateAccessor<N>,
    {
        state.get_with_proof(self.slot_key(key))
    }

    pub fn verify_proof<S: crate::Spec>(
        &self,
        state_root: <S::Storage as Storage>::Root,
        proof: sov_state::StorageProof<<<S as crate::Spec>::Storage as Storage>::Proof>,
    ) -> anyhow::Result<(K, Option<V>)>
where {
        ensure!(
            proof.namespace == N::PROVABLE_NAMESPACE,
            "The provided proof comes from a different namespace. Expected {:?} but found {:?}",
            N::PROVABLE_NAMESPACE,
            proof.namespace
        );

        let (complete_key, value) = <<S as crate::Spec>::Storage>::open_proof(state_root, proof)?;
        let item_key = complete_key.without_prefix();

        let item_key = self
            .codec()
            .key_codec()
            .try_decode(item_key)
            .map_err(|e| anyhow::anyhow!("Failed to decode key from proof: {:?}", e))?;

        let value = value
            .map(|v| {
                self.codec()
                    .value_codec()
                    .try_decode(v.value())
                    .map_err(|e| anyhow::anyhow!("Failed to decode value from proof: {:?}", e))
            })
            .transpose()?;
        Ok((item_key, value))
    }
}

#[cfg(feature = "native")]
impl<K, V, Codec> NamespacedStateMap<User, K, V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<K>,
    K: FromStr + std::fmt::Display,
{
    /// Iterates over all raw values under this map prefix in the user namespace.
    /// Returns per-item decode errors for malformed keys.
    pub fn iter_raw<'a, S>(
        &'a self,
        storage: &'a S,
    ) -> anyhow::Result<Option<impl Iterator<Item = anyhow::Result<(K, Vec<u8>)>> + 'a>>
    where
        S: NativeStorage,
    {
        let prefix = SlotKey::singleton(self.prefix());
        let maybe_iter = storage.maybe_iter_user_values_with_prefix(prefix)?;
        Ok(maybe_iter.map(move |iter| {
            iter.map(move |(key, value)| {
                let decoded_key = self
                    .codec()
                    .key_codec()
                    .try_decode(key.without_prefix())
                    .map_err(|e| anyhow::anyhow!("Failed to decode user map key: {:?}", e))?;
                Ok((decoded_key, value.value().to_vec()))
            })
        }))
    }
}

#[cfg(feature = "native")]
impl<K, V, Codec> NamespacedStateMap<Kernel, K, V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<K>,
    K: FromStr + std::fmt::Display,
{
    /// Iterates over all raw values under this map prefix in the kernel namespace.
    /// Returns per-item decode errors for malformed keys.
    pub fn iter_raw<'a, S>(
        &'a self,
        storage: &'a S,
    ) -> anyhow::Result<Option<impl Iterator<Item = anyhow::Result<(K, Vec<u8>)>> + 'a>>
    where
        S: NativeStorage,
    {
        let prefix = SlotKey::singleton(self.prefix());
        let maybe_iter = storage.maybe_iter_kernel_values_with_prefix(prefix)?;
        Ok(maybe_iter.map(move |iter| {
            iter.map(move |(key, value)| {
                let decoded_key = self
                    .codec()
                    .key_codec()
                    .try_decode(key.without_prefix())
                    .map_err(|e| anyhow::anyhow!("Failed to decode kernel map key: {:?}", e))?;
                Ok((decoded_key, value.value().to_vec()))
            })
        }))
    }
}

#[cfg(feature = "arbitrary")]
impl<'a, N, K, V, Codec> NamespacedStateMap<N, K, V, Codec>
where
    K: arbitrary::Arbitrary<'a> + FromStr + std::fmt::Display,
    V: arbitrary::Arbitrary<'a>,
    Codec: sov_state::StateCodec,
    Codec::KeyCodec: sov_state::StateItemCodec<K>,
    Codec::ValueCodec: sov_state::StateItemCodec<V>,
    N: CompileTimeNamespace,
{
    /// Returns an arbitrary [`StateMap`] instance.
    ///
    /// See the [`arbitrary`] crate for more information.
    pub fn arbitrary_state_map<S>(
        u: &mut arbitrary::Unstructured<'a>,
        working_set: &mut crate::StateCheckpoint<S>,
    ) -> arbitrary::Result<Self>
    where
        S: crate::Spec,
        StateCheckpoint<S>: InfallibleStateReaderAndWriter<N>,
    {
        use arbitrary::Arbitrary;

        let prefix = Prefix::arbitrary(u)?;
        let len = u.arbitrary_len::<(K, V)>()?;
        let codec = Codec::default();
        let map = Self::with_codec(prefix, codec);

        (0..len).try_fold(map, |mut map, _| {
            let key = K::arbitrary(u)?;
            let value = V::arbitrary(u)?;

            map.set(&key, &value, working_set).unwrap_infallible();

            Ok(map)
        })
    }
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use borsh::to_vec as borsh_to_vec;
    use sov_mock_zkvm::MockZkvm;
    use sov_rollup_interface::execution_mode::Native;
    use sov_state::codec::BorshCodec;
    use sov_state::Prefix;
    use sov_test_utils::storage::SimpleJmtStorageManager;
    use sov_test_utils::MockDaSpec;
    use unwrap_infallible::UnwrapInfallible;

    use crate::capabilities::mocks::MockKernel;
    use crate::{AccessoryStateMap, KernelStateMap, StateCheckpoint, StateMap};

    type TestSpec = crate::default_spec::DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn state_map_raw_roundtrip_and_remove() {
        let storage_manager = SimpleJmtStorageManager::new();
        let storage = storage_manager.create_storage();
        let mut state: StateCheckpoint<TestSpec> =
            StateCheckpoint::new(storage, &MockKernel::<TestSpec>::default(), None);

        let prefix = Prefix::new(10, 10);
        let mut map = StateMap::<u64, u32>::with_codec(prefix, BorshCodec);
        let key = 7_u64;

        assert_eq!(map.get_raw(&key, &mut state).unwrap_infallible(), None);

        let raw = vec![3, 1, 4, 1, 5];
        map.set_raw(&key, &raw, &mut state).unwrap_infallible();
        assert_eq!(map.get_raw(&key, &mut state).unwrap_infallible(), Some(raw.clone()));
        assert_eq!(
            map.remove_raw(&key, &mut state).unwrap_infallible(),
            Some(raw.clone())
        );
        assert_eq!(map.get_raw(&key, &mut state).unwrap_infallible(), None);
    }

    #[test]
    fn state_map_raw_and_typed_compatibility() {
        let storage_manager = SimpleJmtStorageManager::new();
        let storage = storage_manager.create_storage();
        let mut state: StateCheckpoint<TestSpec> =
            StateCheckpoint::new(storage, &MockKernel::<TestSpec>::default(), None);

        let prefix = Prefix::new(11, 11);
        let mut map = StateMap::<u64, u32>::with_codec(prefix, BorshCodec);
        let key = 9_u64;

        map.set(&key, &42_u32, &mut state).unwrap_infallible();
        assert_eq!(
            map.get_raw(&key, &mut state).unwrap_infallible(),
            Some(borsh_to_vec(&42_u32).unwrap())
        );

        let typed_from_raw = borsh_to_vec(&100_u32).unwrap();
        map.set_raw(&key, &typed_from_raw, &mut state)
            .unwrap_infallible();
        assert_eq!(map.get(&key, &mut state).unwrap_infallible(), Some(100_u32));
    }

    #[test]
    fn state_map_iter_raw_user_namespace() {
        let storage_manager = SimpleJmtStorageManager::new();
        let storage = storage_manager.create_storage();
        let mut state: StateCheckpoint<TestSpec> =
            StateCheckpoint::new(storage.clone(), &MockKernel::<TestSpec>::default(), None);

        let prefix = Prefix::new(12, 12);
        let mut map = StateMap::<u64, u32>::with_codec(prefix, BorshCodec);
        map.set(&1_u64, &10_u32, &mut state).unwrap_infallible();
        map.set(&2_u64, &20_u32, &mut state).unwrap_infallible();

        // JMT storage does not support prefix iteration.
        assert!(map.iter_raw(&storage).unwrap().is_none());
    }

    #[test]
    fn state_map_iter_raw_kernel_namespace_returns_none_on_jmt() {
        let storage_manager = SimpleJmtStorageManager::new();
        let storage = storage_manager.create_storage();
        let mut state: StateCheckpoint<TestSpec> =
            StateCheckpoint::new(storage.clone(), &MockKernel::<TestSpec>::default(), None);

        let prefix = Prefix::new(13, 13);
        let mut map = KernelStateMap::<u64, u32>::with_codec(prefix, BorshCodec);
        map.set(&1_u64, &10_u32, &mut state).unwrap_infallible();

        assert!(map.iter_raw(&storage).unwrap().is_none());
    }

    #[test]
    fn state_map_iter_raw_from_keys_accessory_namespace() {
        let storage_manager = SimpleJmtStorageManager::new();
        let storage = storage_manager.create_storage();
        let mut state: StateCheckpoint<TestSpec> =
            StateCheckpoint::new(storage, &MockKernel::<TestSpec>::default(), None);

        let prefix = Prefix::new(14, 14);
        let mut map = AccessoryStateMap::<u64, u32>::with_codec(prefix, BorshCodec);
        let mut accessory_state = state.accessory_state();

        map.set(&1_u64, &10_u32, &mut accessory_state)
            .unwrap_infallible();
        map.set_raw(&2_u64, &[0xAA, 0xBB], &mut accessory_state)
            .unwrap_infallible();

        let keys = vec![1_u64, 2_u64, 3_u64];
        let collected = map
            .iter_raw_from_keys(keys, &mut accessory_state)
            .unwrap_infallible()
            .collect::<Vec<_>>();

        assert_eq!(collected[0], (1_u64, Some(borsh_to_vec(&10_u32).unwrap())));
        assert_eq!(collected[1], (2_u64, Some(vec![0xAA, 0xBB])));
        assert_eq!(collected[2], (3_u64, None));
    }
}
