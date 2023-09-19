use std::collections::HashMap;
use std::fmt::Debug;

use sov_first_read_last_write_cache::{CacheKey, CacheValue};
use sov_rollup_interface::stf::Event;
use sov_state::codec::{EncodeKeyLike, StateCodec, StateValueCodec};
use sov_state::storage::{Storage, StorageKey, StorageValue};
use sov_state::{OrderedReadsAndWrites, Prefix, StorageInternalCache};

use crate::{Context, Spec};

/// A working set accumulates reads and writes on top of the underlying DB,
/// automating witness creation.
pub struct Delta<C: Context> {
    inner: <C as Spec>::Storage,
    witness: <<C as Spec>::Storage as Storage>::Witness,
    cache: StorageInternalCache,
}

impl<C: Context> Delta<C> {
    fn new(inner: <C as Spec>::Storage) -> Self {
        Self::with_witness(inner, Default::default())
    }

    fn with_witness(
        inner: <C as Spec>::Storage,
        witness: <<C as Spec>::Storage as Storage>::Witness,
    ) -> Self {
        Self {
            inner,
            witness,
            cache: Default::default(),
        }
    }

    fn freeze(
        &mut self,
    ) -> (
        OrderedReadsAndWrites,
        <<C as Spec>::Storage as Storage>::Witness,
    ) {
        let cache = std::mem::take(&mut self.cache);
        let witness = std::mem::take(&mut self.witness);

        (cache.into(), witness)
    }
}

impl<C: Context> Debug for Delta<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delta").finish()
    }
}

impl<C: Context> StateReaderAndWriter for Delta<C> {
    fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        self.cache.get_or_fetch(key, &self.inner, &self.witness)
    }

    fn set(&mut self, key: &StorageKey, value: StorageValue) {
        self.cache.set(key, value)
    }

    fn delete(&mut self, key: &StorageKey) {
        self.cache.delete(key)
    }
}

type RevertableWrites = HashMap<CacheKey, Option<CacheValue>>;

/// This structure is responsible for storing the `read-write` set.
///
/// A [`StateCheckpoint`] can be obtained from a [`WorkingSet`] in two ways:
///  1. With [`WorkingSet::checkpoint`].
///  2. With [`WorkingSet::revert`].
pub struct StateCheckpoint<C: Context> {
    delta: Delta<C>,
    accessory_delta: AccessoryDelta<C>,
}

impl<C: Context> StateCheckpoint<C> {
    /// Creates a new [`StateCheckpoint`] instance without any changes, backed
    /// by the given [`Storage`].
    pub fn new(inner: <C as Spec>::Storage) -> Self {
        Self {
            delta: Delta::new(inner.clone()),
            accessory_delta: AccessoryDelta::new(inner),
        }
    }

    /// Fetches a value from the underlying storage.
    pub fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        self.delta.get(key)
    }

    /// Creates a new [`StateCheckpoint`] instance without any changes, backed
    /// by the given [`Storage`] and witness.
    pub fn with_witness(
        inner: <C as Spec>::Storage,
        witness: <<C as Spec>::Storage as Storage>::Witness,
    ) -> Self {
        Self {
            delta: Delta::with_witness(inner.clone(), witness),
            accessory_delta: AccessoryDelta::new(inner),
        }
    }

    /// Transforms this [`StateCheckpoint`] back into a [`WorkingSet`].
    pub fn to_revertable(self) -> WorkingSet<C> {
        WorkingSet {
            delta: RevertableWriter::new(self.delta),
            accessory_delta: RevertableWriter::new(self.accessory_delta),
            events: Default::default(),
        }
    }

    /// Extracts ordered reads, writes, and witness from this [`StateCheckpoint`].
    ///
    /// You can then use these to call [`Storage::validate_and_commit`] or some
    /// of the other related [`Storage`] methods. Note that this data is moved
    /// **out** of the [`StateCheckpoint`] i.e. it can't be extracted twice.
    pub fn freeze(
        &mut self,
    ) -> (
        OrderedReadsAndWrites,
        <<C as Spec>::Storage as Storage>::Witness,
    ) {
        self.delta.freeze()
    }

    /// Extracts ordered reads and writes of accessory state from this
    /// [`StateCheckpoint`].
    ///
    /// You can then use these to call
    /// [`Storage::validate_and_commit_with_accessory_update`], together with
    /// the data extracted with [`StateCheckpoint::freeze`].
    pub fn freeze_non_provable(&mut self) -> OrderedReadsAndWrites {
        self.accessory_delta.freeze()
    }
}

struct AccessoryDelta<C: Context> {
    // This inner storage is never accessed inside the zkVM because reads are
    // not allowed, so it can result as dead code.
    #[allow(dead_code)]
    storage: <C as Spec>::Storage,
    writes: RevertableWrites,
}

impl<C: Context> AccessoryDelta<C> {
    fn new(storage: <C as Spec>::Storage) -> Self {
        Self {
            storage,
            writes: Default::default(),
        }
    }

    fn freeze(&mut self) -> OrderedReadsAndWrites {
        let mut reads_and_writes = OrderedReadsAndWrites::default();
        let writes = std::mem::take(&mut self.writes);

        for write in writes {
            reads_and_writes.ordered_writes.push((write.0, write.1));
        }

        reads_and_writes
    }
}

impl<C: Context> StateReaderAndWriter for AccessoryDelta<C> {
    fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        let cache_key = key.to_cache_key();
        if let Some(value) = self.writes.get(&cache_key) {
            return value.clone().map(Into::into);
        }
        self.storage.get_accessory(key)
    }

    fn set(&mut self, key: &StorageKey, value: StorageValue) {
        self.writes
            .insert(key.to_cache_key(), Some(value.into_cache_value()));
    }

    fn delete(&mut self, key: &StorageKey) {
        self.writes.insert(key.to_cache_key(), None);
    }
}

/// This structure contains the read-write set and the events collected during the execution of a transaction.
/// There are two ways to convert it into a StateCheckpoint:
/// 1. By using the checkpoint() method, where all the changes are added to the underlying StateCheckpoint.
/// 2. By using the revert method, where the most recent changes are reverted and the previous `StateCheckpoint` is returned.
pub struct WorkingSet<C: Context> {
    delta: RevertableWriter<Delta<C>>,
    accessory_delta: RevertableWriter<AccessoryDelta<C>>,
    events: Vec<Event>,
}

impl<C: Context> StateReaderAndWriter for WorkingSet<C> {
    fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        self.delta.get(key)
    }

    fn set(&mut self, key: &StorageKey, value: StorageValue) {
        self.delta.set(key, value)
    }

    fn delete(&mut self, key: &StorageKey) {
        self.delta.delete(key)
    }
}

/// A wrapper over [`WorkingSet`] that only allows access to the accessory
/// state (non-JMT state).
pub struct AccessoryWorkingSet<'a, C: Context> {
    ws: &'a mut WorkingSet<C>,
}

impl<'a, C: Context> StateReaderAndWriter for AccessoryWorkingSet<'a, C> {
    fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        if !cfg!(feature = "native") {
            None
        } else {
            self.ws.accessory_delta.get(key)
        }
    }

    fn set(&mut self, key: &StorageKey, value: StorageValue) {
        self.ws.accessory_delta.set(key, value)
    }

    fn delete(&mut self, key: &StorageKey) {
        self.ws.accessory_delta.delete(key)
    }
}

struct RevertableWriter<T> {
    inner: T,
    writes: HashMap<CacheKey, Option<CacheValue>>,
}

impl<T: Debug> Debug for RevertableWriter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevertableWriter")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<T> RevertableWriter<T>
where
    T: StateReaderAndWriter,
{
    fn new(inner: T) -> Self {
        Self {
            inner,
            writes: Default::default(),
        }
    }

    fn commit(mut self) -> T {
        for (k, v) in self.writes.into_iter() {
            if let Some(v) = v {
                self.inner.set(&k.into(), v.into());
            } else {
                self.inner.delete(&k.into());
            }
        }

        self.inner
    }

    fn revert(self) -> T {
        self.inner
    }
}

impl<T: StateReaderAndWriter> StateReaderAndWriter for RevertableWriter<T> {
    fn delete(&mut self, key: &StorageKey) {
        self.writes.insert(key.to_cache_key(), None);
    }

    fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        if let Some(value) = self.writes.get(&key.to_cache_key()) {
            value.as_ref().cloned().map(Into::into)
        } else {
            self.inner.get(key)
        }
    }

    fn set(&mut self, key: &StorageKey, value: StorageValue) {
        self.writes
            .insert(key.to_cache_key(), Some(value.into_cache_value()));
    }
}

impl<C: Context> WorkingSet<C> {
    /// Creates a new [`WorkingSet`] instance backed by the given [`Storage`].
    ///
    /// The witness value is set to [`Default::default`]. Use
    /// [`WorkingSet::with_witness`] to set a custom witness value.
    pub fn new(inner: <C as Spec>::Storage) -> Self {
        StateCheckpoint::new(inner).to_revertable()
    }

    /// Returns a handler for the accessory state (non-JMT state).
    ///
    /// You can use this method when calling getters and setters on accessory
    /// state containers, like [`AccessoryStateMap`](crate::AccessoryStateMap).
    pub fn accessory_state(&mut self) -> AccessoryWorkingSet<C> {
        AccessoryWorkingSet { ws: self }
    }

    /// Creates a new [`WorkingSet`] instance backed by the given [`Storage`]
    /// and a custom witness value.
    pub fn with_witness(
        inner: <C as Spec>::Storage,
        witness: <<C as Spec>::Storage as Storage>::Witness,
    ) -> Self {
        StateCheckpoint::with_witness(inner, witness).to_revertable()
    }

    /// Turns this [`WorkingSet`] into a [`StateCheckpoint`], in preparation for
    /// committing the changes to the underlying [`Storage`] via
    /// [`StateCheckpoint::freeze`].
    pub fn checkpoint(self) -> StateCheckpoint<C> {
        StateCheckpoint {
            delta: self.delta.commit(),
            accessory_delta: self.accessory_delta.commit(),
        }
    }

    /// Reverts the most recent changes to this [`WorkingSet`], returning a pristine
    /// [`StateCheckpoint`] instance.
    pub fn revert(self) -> StateCheckpoint<C> {
        StateCheckpoint {
            delta: self.delta.revert(),
            accessory_delta: self.accessory_delta.revert(),
        }
    }

    /// Adds an event to the working set.
    pub fn add_event(&mut self, key: &str, value: &str) {
        self.events.push(Event::new(key, value));
    }

    /// Extracts all events from this working set.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Returns an immutable slice of all events that have been previously
    /// written to this working set.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Returns an immutable reference to the [`Storage`] instance backing this
    /// working set.
    pub fn backing(&self) -> &<C as Spec>::Storage {
        &self.delta.inner.inner
    }
}

pub(crate) trait StateReaderAndWriter {
    fn get(&mut self, key: &StorageKey) -> Option<StorageValue>;

    fn set(&mut self, key: &StorageKey, value: StorageValue);

    fn delete(&mut self, key: &StorageKey);

    fn set_value<Q, K, V, Codec>(
        &mut self,
        prefix: &Prefix,
        storage_key: &Q,
        value: &V,
        codec: &Codec,
    ) where
        Q: ?Sized,
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
    {
        let storage_key = StorageKey::new(prefix, storage_key, codec.key_codec());
        let storage_value = StorageValue::new(value, codec.value_codec());
        self.set(&storage_key, storage_value);
    }

    fn set_singleton<V, Codec>(&mut self, prefix: &Prefix, value: &V, codec: &Codec)
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateValueCodec<V>,
    {
        let storage_key = StorageKey::singleton(prefix);
        let storage_value = StorageValue::new(value, codec.value_codec());
        self.set(&storage_key, storage_value);
    }

    fn get_decoded<V, Codec>(&mut self, storage_key: &StorageKey, codec: &Codec) -> Option<V>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateValueCodec<V>,
    {
        let storage_value = self.get(storage_key)?;

        Some(
            codec
                .value_codec()
                .decode_value_unwrap(storage_value.value()),
        )
    }

    fn get_value<Q, K, V, Codec>(
        &mut self,
        prefix: &Prefix,
        storage_key: &Q,
        codec: &Codec,
    ) -> Option<V>
    where
        Q: ?Sized,
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
    {
        let storage_key = StorageKey::new(prefix, storage_key, codec.key_codec());
        self.get_decoded(&storage_key, codec)
    }

    fn get_singleton<V, Codec>(&mut self, prefix: &Prefix, codec: &Codec) -> Option<V>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateValueCodec<V>,
    {
        let storage_key = StorageKey::singleton(prefix);
        self.get_decoded(&storage_key, codec)
    }

    fn remove_value<Q, K, V, Codec>(
        &mut self,
        prefix: &Prefix,
        storage_key: &Q,
        codec: &Codec,
    ) -> Option<V>
    where
        Q: ?Sized,
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
        Codec::ValueCodec: StateValueCodec<V>,
    {
        let storage_key = StorageKey::new(prefix, storage_key, codec.key_codec());
        let storage_value = self.get_decoded(&storage_key, codec)?;
        self.delete(&storage_key);
        Some(storage_value)
    }

    fn remove_singleton<V, Codec>(&mut self, prefix: &Prefix, codec: &Codec) -> Option<V>
    where
        Codec: StateCodec,
        Codec::ValueCodec: StateValueCodec<V>,
    {
        let storage_key = StorageKey::singleton(prefix);
        let storage_value = self.get_decoded(&storage_key, codec)?;
        self.delete(&storage_key);
        Some(storage_value)
    }

    fn delete_value<Q, K, Codec>(&mut self, prefix: &Prefix, storage_key: &Q, codec: &Codec)
    where
        Q: ?Sized,
        Codec: StateCodec,
        Codec::KeyCodec: EncodeKeyLike<Q, K>,
    {
        let storage_key = StorageKey::new(prefix, storage_key, codec.key_codec());
        self.delete(&storage_key);
    }

    fn delete_singleton(&mut self, prefix: &Prefix) {
        let storage_key = StorageKey::singleton(prefix);
        self.delete(&storage_key);
    }
}
