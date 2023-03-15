use std::{cell::RefCell, fmt::Debug, ops::DerefMut, rc::Rc};

use crate::{
    internal_cache::StorageInternalCache,
    storage::{StorageKey, StorageValue},
    Storage,
};
use first_read_last_write_cache::cache::CacheLog;
use sovereign_sdk::core::traits::Witness;

// Each transaction operates on its own storage
pub struct WorkingSet<S: Storage> {
    inner: S,
    witness: Rc<S::Witness>,
    cache: Rc<RefCell<StorageInternalCache>>,
}

impl<S: Storage> Clone for WorkingSet<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            witness: self.witness.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<S: Storage> WorkingSet<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            witness: Rc::new(Default::default()),
            cache: Default::default(),
        }
    }

    pub fn with_witness(inner: S, witness: Rc<S::Witness>) -> Self {
        Self {
            inner,
            witness,
            cache: Default::default(),
        }
    }
}

impl<S: Storage> Debug for WorkingSet<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkingSet").finish()
    }
}

impl<S: Storage> WorkingSet<S> {
    pub fn freeze(&self) -> (CacheLog, Rc<S::Witness>) {
        (self.cache.take().into(), self.witness.clone())
    }

    pub fn get(&self, key: StorageKey) -> Option<StorageValue> {
        self.cache
            .borrow_mut()
            .get_or_fetch(key, &self.inner, &self.witness)
    }

    pub fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.cache.borrow_mut().set(key, value)
    }

    pub fn delete(&mut self, key: StorageKey) {
        self.cache.borrow_mut().delete(key)
    }

    pub fn merge(&mut self, rhs: Self) -> Result<(), first_read_last_write_cache::MergeError> {
        // Merge caches
        let rhs_cache = std::mem::take(rhs.cache.borrow_mut().deref_mut());
        self.cache.borrow_mut().merge_left(rhs_cache)?;

        // Merge witnesses
        let rhs_witness = rhs.witness;
        self.witness.merge(&rhs_witness);
        Ok(())
    }
}
