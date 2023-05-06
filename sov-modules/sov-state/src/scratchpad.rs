use std::fmt::Debug;

use crate::{
    internal_cache::StorageInternalCache,
    storage::{StorageKey, StorageValue},
    Prefix, Storage,
};
use first_read_last_write_cache::cache::CacheLog;
use sovereign_sdk::serial::{Decode, Encode};

/// A working set accumulates reads and writes on top of the underlying DB,
/// automating witness creation.
pub struct Delta<S: Storage> {
    inner: S,
    witness: S::Witness,
    cache: StorageInternalCache,
}

/// A wrapper that adds additional reads and writes on top of an underlying Delta.
/// These are handy for implementing operations that might revert on top of an existing
/// working set, without running the risk that the whole working set will be discarded if some particular
/// operation reverts.
pub struct RevertableDelta<S: Storage> {
    inner: Delta<S>,
    cache: StorageInternalCache,
}

impl<S: Storage> Debug for RevertableDelta<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevertableDelta")
            .field("inner", &self.inner)
            .finish()
    }
}

/// A read-write set which can be committed as a unit
pub enum WorkingSet<S: Storage> {
    Standard(Delta<S>),
    Revertable(RevertableDelta<S>),
}

impl<S: Storage> WorkingSet<S> {
    pub fn new(inner: S) -> Self {
        Self::Standard(Delta::new(inner))
    }

    pub fn with_witness(inner: S, witness: S::Witness) -> Self {
        Self::Standard(Delta::with_witness(inner, witness))
    }

    pub fn to_revertable(self) -> Self {
        match self {
            WorkingSet::Standard(delta) => WorkingSet::Revertable(delta.get_revertable_wrapper()),
            WorkingSet::Revertable(_) => self,
        }
    }

    pub fn commit(self) -> Self {
        match self {
            s @ WorkingSet::Standard(_) => s,
            WorkingSet::Revertable(revertable) => WorkingSet::Standard(revertable.commit()),
        }
    }

    pub fn revert(self) -> Self {
        match self {
            s @ WorkingSet::Standard(_) => s,
            WorkingSet::Revertable(revertable) => WorkingSet::Standard(revertable.revert()),
        }
    }

    pub fn get(&mut self, key: StorageKey) -> Option<StorageValue> {
        println!("Searching for key {:?} in cache", hex::encode(key.as_ref()));
        match self {
            WorkingSet::Standard(s) => s.get(key),
            WorkingSet::Revertable(s) => s.get(key),
        }
    }

    pub fn set(&mut self, key: StorageKey, value: StorageValue) {
        match self {
            WorkingSet::Standard(s) => s.set(key, value),
            WorkingSet::Revertable(s) => s.set(key, value),
        }
    }

    pub fn delete(&mut self, key: StorageKey) {
        match self {
            WorkingSet::Standard(s) => s.delete(key),
            WorkingSet::Revertable(s) => s.delete(key),
        }
    }

    pub fn freeze(&mut self) -> (StorageInternalCache, S::Witness) {
        match self {
            WorkingSet::Standard(delta) => delta.freeze(),
            WorkingSet::Revertable(_) => todo!(),
        }
    }

    pub fn backing(&self) -> &S {
        match self {
            WorkingSet::Standard(delta) => &delta.inner,
            WorkingSet::Revertable(revertable) => &revertable.inner.inner,
        }
    }
}

impl<S: Storage> RevertableDelta<S> {
    fn get(&mut self, key: StorageKey) -> Option<StorageValue> {
        match self.cache.try_get(key.clone()) {
            first_read_last_write_cache::cache::ValueExists::Yes(val) => {
                println!("Key found in *revertable* cache");
                val.map(StorageValue::new_from_cache_value)
            }
            first_read_last_write_cache::cache::ValueExists::No => {
                println!("Key not found in *revertable* cache. Checking inner cache.");
                self.inner.get(key)
            }
        }
    }

    fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.cache.set(key, value)
    }

    fn delete(&mut self, key: StorageKey) {
        self.cache.delete(key)
    }
}

impl<S: Storage> RevertableDelta<S> {
    fn commit(self) -> Delta<S> {
        let mut inner = self.inner;

        inner
            .cache
            .merge_left(self.cache)
            .expect("caches must be consistent");

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
            cache: Default::default(),
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
        println!("Checking non-revertable cache");
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
    fn freeze(&mut self) -> (StorageInternalCache, S::Witness) {
        let cache = std::mem::take(&mut self.cache);
        let witness = std::mem::take(&mut self.witness);

        (cache.into(), witness)
    }
}

impl<S: Storage> WorkingSet<S> {
    pub(crate) fn set_value<K: Encode, V: Encode>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
        value: V,
    ) {
        let storage_key = StorageKey::new(prefix, storage_key);
        let storage_value = StorageValue::new(value);
        self.set(storage_key, storage_value);
    }

    pub(crate) fn get_value<K: Encode, V: Decode>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
    ) -> Option<V> {
        let storage_key = StorageKey::new(prefix, storage_key);
        self.get_decoded(storage_key)
    }

    pub(crate) fn remove_value<K: Encode, V: Decode>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
    ) -> Option<V> {
        let storage_key = StorageKey::new(prefix, storage_key);
        let storage_value = self.get_decoded(storage_key.clone())?;
        self.delete(storage_key);
        Some(storage_value)
    }

    pub(crate) fn delete_value<K: Encode>(&mut self, prefix: &Prefix, storage_key: &K) {
        let storage_key = StorageKey::new(prefix, storage_key);
        self.delete(storage_key);
    }

    fn get_decoded<V: Decode>(&mut self, storage_key: StorageKey) -> Option<V> {
        let storage_value = self.get(storage_key)?;

        // It is ok to panic here. Deserialization problem means that something is terribly wrong.
        Some(
            V::decode(&mut storage_value.value())
                .unwrap_or_else(|e| panic!("Unable to deserialize storage value {e:?}")),
        )
    }
}
