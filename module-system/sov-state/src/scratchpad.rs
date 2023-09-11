use std::collections::HashMap;
use std::fmt::Debug;

use sov_first_read_last_write_cache::{CacheKey, CacheValue};
use sov_rollup_interface::stf::Event;

use crate::codec::{EncodeKeyLike, StateCodec, StateValueCodec};
use crate::internal_cache::{OrderedReadsAndWrites, StorageInternalCache};
use crate::storage::{StorageKey, StorageValue};
use crate::{Prefix, Storage};

/// A working set accumulates reads and writes on top of the underlying DB,
/// automating witness creation.
pub struct Delta<S: Storage> {
    inner: S,
    witness: S::Witness,
    cache: StorageInternalCache,
}

impl<S: Storage> Delta<S> {
    fn new(inner: S) -> Self {
        Self::with_witness(inner, Default::default())
    }

    fn with_witness(inner: S, witness: S::Witness) -> Self {
        Self {
            inner,
            witness,
            cache: Default::default(),
        }
    }

    fn freeze(&mut self) -> (OrderedReadsAndWrites, S::Witness) {
        let cache = std::mem::take(&mut self.cache);
        let witness = std::mem::take(&mut self.witness);

        (cache.into(), witness)
    }
}

impl<S: Storage> Debug for Delta<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delta").finish()
    }
}

impl<S: Storage> StateReaderAndWriter for Delta<S> {
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

/// This structure is responsible for storing the `read-write` set
/// and is obtained from the `WorkingSet` by using either the `commit` or `revert` method.
pub struct StateCheckpoint<S: Storage> {
    delta: Delta<S>,
    accessory_delta: AccessoryDelta<S>,
}

impl<S: Storage> StateCheckpoint<S> {
    pub fn new(inner: S) -> Self {
        Self {
            delta: Delta::new(inner.clone()),
            accessory_delta: AccessoryDelta::new(inner),
        }
    }

    pub fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        self.delta.get(key)
    }

    pub fn with_witness(inner: S, witness: S::Witness) -> Self {
        Self {
            delta: Delta::with_witness(inner.clone(), witness),
            accessory_delta: AccessoryDelta::new(inner),
        }
    }

    pub fn to_revertable(self) -> WorkingSet<S> {
        WorkingSet {
            delta: RevertableWriter::new(self.delta),
            accessory_delta: RevertableWriter::new(self.accessory_delta),
            events: Default::default(),
        }
    }

    pub fn freeze(&mut self) -> (OrderedReadsAndWrites, S::Witness) {
        self.delta.freeze()
    }

    pub fn freeze_non_provable(&mut self) -> OrderedReadsAndWrites {
        self.accessory_delta.freeze()
    }
}

struct AccessoryDelta<S: Storage> {
    // This inner storage is never accessed inside the zkVM because reads are
    // not allowed, so it can result as dead code.
    #[allow(dead_code)]
    storage: S,
    writes: RevertableWrites,
}

impl<S: Storage> AccessoryDelta<S> {
    fn new(storage: S) -> Self {
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

impl<S: Storage> StateReaderAndWriter for AccessoryDelta<S> {
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
pub struct WorkingSet<S: Storage> {
    delta: RevertableWriter<Delta<S>>,
    accessory_delta: RevertableWriter<AccessoryDelta<S>>,
    events: Vec<Event>,
}

impl<S: Storage> StateReaderAndWriter for WorkingSet<S> {
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
pub struct AccessoryWorkingSet<'a, S: Storage> {
    ws: &'a mut WorkingSet<S>,
}

impl<'a, S: Storage> StateReaderAndWriter for AccessoryWorkingSet<'a, S> {
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

impl<S: Storage> WorkingSet<S> {
    pub fn new(inner: S) -> Self {
        StateCheckpoint::new(inner).to_revertable()
    }

    pub fn accessory_state(&mut self) -> AccessoryWorkingSet<S> {
        AccessoryWorkingSet { ws: self }
    }

    pub fn with_witness(inner: S, witness: S::Witness) -> Self {
        StateCheckpoint::with_witness(inner, witness).to_revertable()
    }

    pub fn checkpoint(self) -> StateCheckpoint<S> {
        StateCheckpoint {
            delta: self.delta.commit(),
            accessory_delta: self.accessory_delta.commit(),
        }
    }

    pub fn revert(self) -> StateCheckpoint<S> {
        StateCheckpoint {
            delta: self.delta.revert(),
            accessory_delta: self.accessory_delta.revert(),
        }
    }

    pub fn add_event(&mut self, key: &str, value: &str) {
        self.events.push(Event::new(key, value));
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn backing(&self) -> &S {
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
