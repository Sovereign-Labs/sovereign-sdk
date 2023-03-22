use std::fmt::Debug;

use crate::{
    internal_cache::StorageInternalCache,
    storage::{StorageKey, StorageValue},
    Prefix, Storage,
};
use first_read_last_write_cache::cache::CacheLog;
use sovereign_sdk::{
    core::traits::Witness,
    serial::{Decode, Encode},
};

/// A working set accumulates reads and writes on top of the underlying DB,
/// automating witness creation.
pub struct Delta<S: Storage> {
    inner: S,
    witness: S::Witness,
    cache: StorageInternalCache,
}

/// A wrapper that adds additional reads and writes on top of an underlying Delta.
/// These are handly for implementing operations that might revert on top of an existing
/// working set, without running the risk that the whole working set will be discarded if some particular
/// operation reverts.
pub struct RevertableDelta<S: Storage> {
    inner: Delta<S>,
    witness: S::Witness,
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

    pub fn to_revertable(self) -> WorkingSet<S> {
        match self {
            WorkingSet::Standard(delta) => WorkingSet::Revertable(delta.get_revertable_wrapper()),
            r @ WorkingSet::Revertable(_) => r,
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

    pub fn freeze(&mut self) -> (CacheLog, S::Witness) {
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
                val.map(StorageValue::new_from_cache_value)
            }
            first_read_last_write_cache::cache::ValueExists::No => {
                self.inner.get_with_witness(key, &self.witness)
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

        inner.witness.merge(&self.witness);
        inner
    }

    fn revert(self) -> Delta<S> {
        let mut inner = self.inner;

        inner
            .cache
            .merge_reads_left(self.cache)
            .expect("caches must be consistent");

        inner.witness.merge(&self.witness);
        inner
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
        self.get_revertable_wrapper_with_witness(Default::default())
    }

    fn get_revertable_wrapper_with_witness(self, witness: S::Witness) -> RevertableDelta<S> {
        RevertableDelta {
            inner: self,
            witness,
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
    fn freeze(&mut self) -> (CacheLog, S::Witness) {
        let cache = std::mem::take(&mut self.cache);
        let witness = std::mem::take(&mut self.witness);

        (cache.into(), witness)
    }

    fn get_with_witness(&mut self, key: StorageKey, witness: &S::Witness) -> Option<StorageValue> {
        self.cache.get_or_fetch(key, &self.inner, witness)
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
        let storage_value = self.get(storage_key)?;

        // It is ok to panic here. Deserialization problem means that something is terribly wrong.
        Some(
            V::decode(&mut storage_value.value())
                .unwrap_or_else(|e| panic!("Unable to deserialize storage value {e:?}")),
        )
    }

    pub(crate) fn remove_value<K: Encode, V: Decode>(
        &mut self,
        prefix: &Prefix,
        storage_key: &K,
    ) -> Option<V> {
        let storage_value = self.get_value(prefix, storage_key)?;
        let storage_key = StorageKey::new(prefix, storage_key);
        self.delete(storage_key);
        Some(storage_value)
    }
}
