use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

use sov_first_read_last_write_cache::{CacheKey, CacheValue};
use sov_rollup_interface::stf::Event;

use crate::codec::StateValueCodec;
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

type RevertableWrites = HashMap<CacheKey, Option<CacheValue>>;

/// A wrapper that adds additional writes on top of an underlying Delta.
/// These are handy for implementing operations that might revert on top of an existing
/// working set, without running the risk that the whole working set will be discarded if some particular
/// operation reverts.
///
/// All reads are recorded in the underlying delta, because even reverted transactions have to be proven to have
/// executed against the correct state. (If the state was different, the transaction may not have reverted.)
struct RevertableDelta<S: Storage> {
    /// The inner (non-revertable) delta
    inner: Delta<S>,
    /// A cache containing the most recent values written. Reads are first checked
    /// against this map, and if the key is not present, the underlying Delta is checked.
    writes: RevertableWrites,
}

impl<S: Storage> Debug for RevertableDelta<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevertableDelta")
            .field("inner", &self.inner)
            .finish()
    }
}

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
            delta: self.delta.get_revertable_wrapper(),
            accessory_delta: self.accessory_delta.get_revertable_wrapper(),
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

    fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        let cache_key = key.to_cache_key();
        if let Some(value) = self.writes.get(&cache_key) {
            return value.clone().map(Into::into);
        }
        self.storage.get_accessory(key)
    }

    fn freeze(&mut self) -> OrderedReadsAndWrites {
        let mut reads_and_writes = OrderedReadsAndWrites::default();
        let writes = std::mem::take(&mut self.writes);

        for write in writes {
            reads_and_writes.ordered_writes.push((write.0, write.1));
        }

        reads_and_writes
    }

    fn set(&mut self, key: &StorageKey, value: StorageValue) {
        self.writes
            .insert(key.to_cache_key(), Some(value.into_cache_value()));
    }

    fn delete(&mut self, key: &StorageKey) {
        self.writes.insert(key.to_cache_key(), None);
    }

    fn get_revertable_wrapper(self) -> RevertableAccessoryDelta<S> {
        RevertableAccessoryDelta::new(self)
    }
}

struct RevertableAccessoryDelta<S: Storage> {
    delta: AccessoryDelta<S>,
    writes: RevertableWrites,
}

impl<S: Storage> RevertableAccessoryDelta<S> {
    fn new(delta: AccessoryDelta<S>) -> Self {
        Self {
            delta,
            writes: Default::default(),
        }
    }

    fn get(&mut self, key: &StorageKey) -> Option<StorageValue> {
        let cache_key = key.to_cache_key();
        if let Some(value) = self.writes.get(&cache_key) {
            return value.clone().map(Into::into);
        }
        self.delta.get(key)
    }

    fn set(&mut self, key: &StorageKey, value: StorageValue) {
        self.writes
            .insert(key.to_cache_key(), Some(value.into_cache_value()));
    }

    fn revert(self) -> AccessoryDelta<S> {
        self.delta
    }

    fn commit(mut self) -> AccessoryDelta<S> {
        for (k, v) in self.writes.into_iter() {
            if let Some(v) = v {
                self.delta.set(&k.into(), v.into());
            } else {
                self.delta.delete(&k.into());
            }
        }

        self.delta
    }
}

/// This structure contains the read-write set and the events collected during the execution of a transaction.
/// There are two ways to convert it into a StateCheckpoint:
/// 1. By using the checkpoint() method, where all the changes are added to the underlying StateCheckpoint.
/// 2. By using the revert method, where the most recent changes are reverted and the previous `StateCheckpoint` is returned.
pub struct WorkingSet<S: Storage> {
    delta: RevertableDelta<S>,
    accessory_delta: RevertableAccessoryDelta<S>,
    events: Vec<Event>,
}

impl<S: Storage> WorkingSet<S> {
    pub fn new(inner: S) -> Self {
        StateCheckpoint::new(inner).to_revertable()
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

    pub fn set_unprovable(&mut self, key: StorageKey, value: StorageValue) {
        println!("setting unprovable, key: {:?}, value: {:?}", key, value);
        self.accessory_delta.set(&key, value)
    }

    #[cfg(feature = "native")]
    pub fn get_unprovable(&mut self, key: StorageKey) -> Option<StorageValue> {
        println!("getting unprovable in working set, key: {:?}", key);
        self.accessory_delta.get(&key)
    }

    pub(crate) fn get(&mut self, key: StorageKey) -> Option<StorageValue> {
        self.delta.get(key)
    }

    pub(crate) fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.delta.set(key, value)
    }

    pub(crate) fn delete(&mut self, key: StorageKey) {
        self.delta.delete(key)
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

impl<S: Storage> WorkingSet<S> {
    pub(crate) fn set_value<K, V, VC>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
        value: &V,
        codec: &VC,
    ) where
        K: Hash + Eq + ?Sized,
        VC: StateValueCodec<V>,
    {
        let storage_key = StorageKey::new(prefix, storage_key);
        let storage_value = StorageValue::new(value, codec);
        self.set(storage_key, storage_value);
    }

    pub(crate) fn get_value<K, V, VC>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
        codec: &VC,
    ) -> Option<V>
    where
        K: Hash + Eq + ?Sized,
        VC: StateValueCodec<V>,
    {
        let storage_key = StorageKey::new(prefix, storage_key);
        self.get_decoded(storage_key, codec)
    }

    pub(crate) fn remove_value<K, V, VC>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
        codec: &VC,
    ) -> Option<V>
    where
        K: Hash + Eq + ?Sized,
        VC: StateValueCodec<V>,
    {
        let storage_key = StorageKey::new(prefix, storage_key);
        let storage_value = self.get_decoded(storage_key.clone(), codec)?;
        self.delete(storage_key);
        Some(storage_value)
    }

    pub(crate) fn delete_value<K>(&mut self, prefix: &Prefix, storage_key: &K)
    where
        K: Hash + Eq + ?Sized,
    {
        let storage_key = StorageKey::new(prefix, storage_key);
        self.delete(storage_key);
    }

    fn get_decoded<V, VC>(&mut self, storage_key: StorageKey, codec: &VC) -> Option<V>
    where
        VC: StateValueCodec<V>,
    {
        let storage_value = self.get(storage_key)?;

        Some(codec.decode_value_unwrap(storage_value.value()))
    }
}

impl<S: Storage> RevertableDelta<S> {
    fn get(&mut self, key: StorageKey) -> Option<StorageValue> {
        let key = key.to_cache_key();
        if let Some(value) = self.writes.get(&key) {
            return value.clone().map(Into::into);
        }
        self.inner.get(&key.into())
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.writes
            .insert(key.to_cache_key(), Some(value.into_cache_value()));
    }

    fn delete(&mut self, key: StorageKey) {
        self.writes.insert(key.to_cache_key(), None);
    }
}

impl<S: Storage> RevertableDelta<S> {
    fn commit(self) -> Delta<S> {
        let mut inner = self.inner;

        for (k, v) in self.writes.into_iter() {
            if let Some(v) = v {
                inner.set(&k.into(), v.into());
            } else {
                inner.delete(&k.into());
            }
        }

        inner
    }

    fn revert(self) -> Delta<S> {
        self.inner
    }
}

impl<S: Storage> Delta<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            witness: Default::default(),
            cache: Default::default(),
        }
    }

    fn with_witness(inner: S, witness: S::Witness) -> Self {
        Self {
            inner,
            witness,
            cache: Default::default(),
        }
    }

    fn get_revertable_wrapper(self) -> RevertableDelta<S> {
        RevertableDelta {
            inner: self,
            writes: Default::default(),
        }
    }
}

impl<S: Storage> Debug for Delta<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delta").finish()
    }
}

impl<S: Storage> Delta<S> {
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

impl<S: Storage> Delta<S> {
    fn freeze(&mut self) -> (OrderedReadsAndWrites, S::Witness) {
        let cache = std::mem::take(&mut self.cache);
        let witness = std::mem::take(&mut self.witness);

        (cache.into(), witness)
    }
}
