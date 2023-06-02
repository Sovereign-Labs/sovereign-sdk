use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::stf::Event;
use std::{collections::HashMap, fmt::Debug};

use crate::{
    internal_cache::{OrderedReadsAndWrites, StorageInternalCache},
    storage::{StorageKey, StorageValue},
    Prefix, Storage,
};
use sov_first_read_last_write_cache::{CacheKey, CacheValue};

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

pub struct CommitedWorkinSet<S: Storage> {
    delta: Delta<S>,
}

impl<S: Storage> CommitedWorkinSet<S> {
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

// TODO rename it to WorkingSet
pub struct WorkingSet<S: Storage> {
    delta: RevertableDelta<S>,
    events: Vec<Event>,
}

impl<S: Storage> WorkingSet<S> {
    pub fn commit(self) -> CommitedWorkinSet<S> {
        CommitedWorkinSet {
            delta: self.delta.commit(),
        }
    }

    pub fn revert(self) -> CommitedWorkinSet<S> {
        CommitedWorkinSet {
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

    //TODO do we need this function, probably not
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

/*
/// A read-write set which can be committed as a unit
enum ReadWriteSet<S: Storage> {
    Standard(Delta<S>),
    Revertable(RevertableDelta<S>),
}

/// This structure holds the read-write set and the events gathered during the execution of a transaction.
pub struct WorkingSet<S: Storage> {
    read_write_set: ReadWriteSet<S>,
    events: Vec<Event>,
}

impl<S: Storage> WorkingSet<S> {
    pub fn new(inner: S) -> Self {
        Self {
            read_write_set: ReadWriteSet::Standard(Delta::new(inner)),
            events: Default::default(),
        }
    }

    pub fn with_witness(inner: S, witness: S::Witness) -> Self {
        Self {
            read_write_set: ReadWriteSet::Standard(Delta::with_witness(inner, witness)),
            events: Default::default(),
        }
    }

    pub fn to_revertable(self) -> Self {
        let read_write_set = match self.read_write_set {
            ReadWriteSet::Standard(delta) => {
                ReadWriteSet::Revertable(delta.get_revertable_wrapper())
            }
            ReadWriteSet::Revertable(_) => self.read_write_set,
        };

        Self {
            read_write_set,
            events: self.events,
        }
    }

    pub fn commit(self) -> Self {
        let read_write_set = match self.read_write_set {
            s @ ReadWriteSet::Standard(_) => s,
            ReadWriteSet::Revertable(revertable) => ReadWriteSet::Standard(revertable.commit()),
        };

        Self {
            read_write_set,
            events: self.events,
        }
    }

    pub fn revert(self) -> Self {
        let read_write_set = match self.read_write_set {
            s @ ReadWriteSet::Standard(_) => s,
            ReadWriteSet::Revertable(revertable) => ReadWriteSet::Standard(revertable.revert()),
        };
        Self {
            read_write_set,
            // The `revert` removes all events associated with the transaction
            events: Vec::default(),
        }
    }

    pub(crate) fn get(&mut self, key: StorageKey) -> Option<StorageValue> {
        match &mut self.read_write_set {
            ReadWriteSet::Standard(s) => s.get(key),
            ReadWriteSet::Revertable(s) => s.get(key),
        }
    }

    pub(crate) fn set(&mut self, key: StorageKey, value: StorageValue) {
        match &mut self.read_write_set {
            ReadWriteSet::Standard(s) => s.set(key, value),
            ReadWriteSet::Revertable(s) => s.set(key, value),
        }
    }

    pub(crate) fn delete(&mut self, key: StorageKey) {
        match &mut self.read_write_set {
            ReadWriteSet::Standard(s) => s.delete(key),
            ReadWriteSet::Revertable(s) => s.delete(key),
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

    pub fn freeze(&mut self) -> (OrderedReadsAndWrites, S::Witness) {
        match &mut self.read_write_set {
            ReadWriteSet::Standard(delta) => delta.freeze(),
            ReadWriteSet::Revertable(_) => todo!(),
        }
    }

    pub fn backing(&self) -> &S {
        match &self.read_write_set {
            ReadWriteSet::Standard(delta) => &delta.inner,
            ReadWriteSet::Revertable(revertable) => &revertable.inner.inner,
        }
    }
}
*/
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
/*
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
*/
