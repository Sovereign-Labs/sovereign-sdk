use std::{cell::RefCell, fmt::Debug, ops::DerefMut, rc::Rc};

use crate::{
    internal_cache::StorageInternalCache,
    storage::{StorageKey, StorageValue},
    Storage,
};
use first_read_last_write_cache::cache::CacheLog;
use sovereign_sdk::core::traits::Witness;

/// A working set accumulates reads and writes on top of the underlying DB,
/// automating witness creation.
pub struct Delta<S: Storage> {
    inner: S,
    witness: Rc<S::Witness>,
    cache: Rc<RefCell<StorageInternalCache>>,
}

/// A wrapper that adds additional reads and writes on top of an underlying Delta.
/// These are handly for implementing operations that might revert on top of an existing
/// working set, without running the risk that the whole working set will be discarded if some particular
/// operation reverts.
pub struct RevertableDelta<S: Storage> {
    inner: Rc<RefCell<Option<Delta<S>>>>,
    witness: Rc<S::Witness>,
    cache: Rc<RefCell<StorageInternalCache>>,
}

impl<S: Storage> Debug for RevertableDelta<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevertableDelta")
            .field("inner", &self.inner)
            .finish()
    }
}

/// A read-write set which can be committed as a unit
#[derive(Clone, Debug)]
pub enum WorkingSet<S: Storage> {
    Standard(Delta<S>),
    Revertable(RevertableDelta<S>),
}

impl<S: Storage> WorkingSet<S> {
    pub fn new(inner: S) -> Self {
        Self::Standard(Delta::new(inner))
    }

    pub fn with_witness(inner: S, witness: Rc<S::Witness>) -> Self {
        Self::Standard(Delta::with_witness(inner, witness))
    }

    pub fn to_revertable(&mut self) {
        match self {
            WorkingSet::Standard(delta) => {
                *self = WorkingSet::Revertable(delta.get_revertable_wrapper())
            }
            WorkingSet::Revertable(_) => {}
        }
    }

    pub fn commit(&mut self) {
        match self {
            WorkingSet::Standard(_) => {}
            WorkingSet::Revertable(revertable) => *self = WorkingSet::Standard(revertable.commit()),
        }
    }

    pub fn revert(&mut self) {
        match self {
            WorkingSet::Standard(_) => {}
            WorkingSet::Revertable(revertable) => *self = WorkingSet::Standard(revertable.revert()),
        }
    }

    pub fn get(&self, key: StorageKey) -> Option<StorageValue> {
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

    pub fn freeze(&self) -> (CacheLog, Rc<S::Witness>) {
        match self {
            WorkingSet::Standard(delta) => delta.freeze(),
            WorkingSet::Revertable(_) => todo!(),
        }
    }

    pub fn backing(&self) -> S {
        match self {
            WorkingSet::Standard(delta) => delta.inner.clone(),
            WorkingSet::Revertable(revertable) => revertable
                .inner
                .borrow()
                .as_ref()
                .expect("Inner must exist")
                .inner
                .clone(),
        }
    }
}

impl<S: Storage> Clone for RevertableDelta<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            witness: self.witness.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<S: Storage> RevertableDelta<S> {
    pub fn get(&self, key: StorageKey) -> Option<StorageValue> {
        match self.cache.borrow().try_get(key.clone()) {
            first_read_last_write_cache::cache::ValueExists::Yes(val) => {
                val.map(StorageValue::new_from_cache_value)
            }
            first_read_last_write_cache::cache::ValueExists::No => self
                .inner
                .borrow()
                .as_ref()
                .expect("inner delta must exist")
                .get_with_witness(key, &self.witness),
        }
    }

    pub fn set(&mut self, key: StorageKey, value: StorageValue) {
        self.cache.borrow_mut().set(key, value)
    }

    pub fn delete(&mut self, key: StorageKey) {
        self.cache.borrow_mut().delete(key)
    }
}

impl<S: Storage> RevertableDelta<S> {
    pub fn commit(&mut self) -> Delta<S> {
        let inner = self
            .inner
            .borrow_mut()
            .take()
            .expect("Only one revertable delta may be merged");

        inner
            .cache
            .borrow_mut()
            .merge_left(std::mem::take(&mut self.cache.as_ref().borrow_mut()))
            .expect("caches must be consistent");

        inner.witness.merge(self.witness.as_ref());
        inner
    }

    pub fn revert(&mut self) -> Delta<S> {
        let inner = self
            .inner
            .borrow_mut()
            .take()
            .expect("Only one revertable delta may be merged");

        inner
            .cache
            .borrow_mut()
            .merge_reads_left(std::mem::take(&mut self.cache.as_ref().borrow_mut()))
            .expect("caches must be consistent");

        inner.witness.merge(self.witness.as_ref());
        inner
    }
}

impl<S: Storage> Clone for Delta<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            witness: self.witness.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<S: Storage> Delta<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            witness: Rc::new(Default::default()),
            cache: Default::default(),
        }
    }

    pub fn get_revertable_wrapper(&self) -> RevertableDelta<S> {
        self.get_revertable_wrapper_with_witness(Default::default())
    }

    pub fn get_revertable_wrapper_with_witness(
        &self,
        witness: Rc<S::Witness>,
    ) -> RevertableDelta<S> {
        RevertableDelta {
            inner: Rc::new(RefCell::new(Some(self.clone()))),
            witness: witness,
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

impl<S: Storage> Debug for Delta<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Delta").finish()
    }
}

impl<S: Storage> Delta<S> {
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
}

impl<S: Storage> Delta<S> {
    pub fn freeze(&self) -> (CacheLog, Rc<S::Witness>) {
        (self.cache.take().into(), self.witness.clone())
    }

    fn get_with_witness(&self, key: StorageKey, witness: &S::Witness) -> Option<StorageValue> {
        self.cache
            .borrow_mut()
            .get_or_fetch(key, &self.inner, witness)
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
