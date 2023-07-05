use std::collections::HashMap;
use std::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_first_read_last_write_cache::{CacheKey, CacheValue};
use sov_rollup_interface::stf::Event;

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
    writes: HashMap<CacheKey, Option<CacheValue>>,
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
}

impl<S: Storage> StateCheckpoint<S> {
    pub fn new(inner: S) -> Self {
        Self {
            delta: Delta::new(inner),
        }
    }

    pub fn with_witness(inner: S, witness: S::Witness) -> Self {
        Self {
            delta: Delta::with_witness(inner, witness),
        }
    }

    pub fn to_revertable(self) -> WorkingSet<S> {
        WorkingSet {
            delta: self.delta.get_revertable_wrapper(),
            events: Default::default(),
        }
    }

    pub fn freeze(&mut self) -> (OrderedReadsAndWrites, S::Witness) {
        self.delta.freeze()
    }
}

/// This structure contains the read-write set and the events collected during the execution of a transaction.
/// There are two ways to convert it into a StateCheckpoint:
/// 1. By using the checkpoint() method, where all the changes are added to the underlying StateCheckpoint.
/// 2. By using the revert method, where the most recent changes are reverted and the previous `StateCheckpoint` is returned.
pub struct WorkingSet<S: Storage> {
    delta: RevertableDelta<S>,
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
        }
    }

    pub fn revert(self) -> StateCheckpoint<S> {
        StateCheckpoint {
            delta: self.delta.revert(),
        }
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

    #[cfg(test)]
    pub fn backing(&self) -> &S {
        &self.delta.inner.inner
    }
}

impl<S: Storage> WorkingSet<S> {
    pub(crate) fn set_value<K: BorshSerialize, V: BorshSerialize>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
        value: &V,
    ) {
        let storage_key = StorageKey::new(prefix, storage_key);
        let storage_value = StorageValue::new(value);
        self.set(storage_key, storage_value);
    }

    pub(crate) fn get_value<K: BorshSerialize, V: BorshDeserialize>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
    ) -> Option<V> {
        let storage_key = StorageKey::new(prefix, storage_key);
        self.get_decoded(storage_key)
    }

    pub(crate) fn remove_value<K: BorshSerialize, V: BorshDeserialize>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
    ) -> Option<V> {
        let storage_key = StorageKey::new(prefix, storage_key);
        let storage_value = self.get_decoded(storage_key.clone())?;
        self.delete(storage_key);
        Some(storage_value)
    }

    pub(crate) fn delete_value<K: BorshSerialize>(&mut self, prefix: &Prefix, storage_key: &K) {
        let storage_key = StorageKey::new(prefix, storage_key);
        self.delete(storage_key);
    }

    fn get_decoded<V: BorshDeserialize>(&mut self, storage_key: StorageKey) -> Option<V> {
        let storage_value = self.get(storage_key)?;

        // It is ok to panic here. Deserialization problem means that something is terribly wrong.
        Some(
            V::deserialize_reader(&mut storage_value.value())
                .unwrap_or_else(|e| panic!("Unable to deserialize storage value {e:?}")),
        )
    }
}

impl<S: Storage> RevertableDelta<S> {
    fn get(&mut self, key: StorageKey) -> Option<StorageValue> {
        let key = key.as_cache_key();
        if let Some(value) = self.writes.get(&key) {
            return value.clone().map(StorageValue::new_from_cache_value);
        }
        self.inner.get(key.into())
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.writes
            .insert(key.as_cache_key(), Some(value.as_cache_value()));
    }

    fn delete(&mut self, key: StorageKey) {
        self.writes.insert(key.as_cache_key(), None);
    }
}

impl<S: Storage> RevertableDelta<S> {
    fn commit(self) -> Delta<S> {
        let mut inner = self.inner;

        for (k, v) in self.writes.into_iter() {
            if let Some(v) = v {
                inner.set(k.into(), StorageValue::new_from_cache_value(v));
            } else {
                inner.delete(k.into());
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
    fn get(&mut self, key: StorageKey) -> Option<StorageValue> {
        self.cache.get_or_fetch(key, &self.inner, &self.witness)
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
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
