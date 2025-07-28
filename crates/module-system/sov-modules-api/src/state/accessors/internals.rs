use core::fmt;
use std::collections::HashMap;

use sov_state::{
    namespaces, AccessSize, IsValueCached, Namespace, ProvableStorageCache, SlotKey, SlotValue,
    StateAccesses, Storage,
};

use super::checkpoints::ChangeSet;
use super::temp_cache::{CacheLookup, TempCache};
use super::UniversalStateAccessor;
use crate::state::traits::PerBlockCache;

/// A [`Delta`] is a diff over an underlying [`Storage`] instance. When queried, it first checks
/// whether the value is in its local cache and, if so, returns it. Otherwise, it queries the
/// underlying storage for the requested key, adds it to the Witness, and populates the value into
/// its own local cache before returning.
///
/// Writes are always performed on the local cache, and are only committed to the underlying storage
/// when the `Delta` is frozen.
pub(super) struct Delta<S: Storage> {
    pub(super) inner: S,
    witness: S::Witness,
    pub(crate) kernel_cache: ProvableStorageCache<namespaces::Kernel>,
    pub(crate) user_cache: ProvableStorageCache<namespaces::User>,
    pub(crate) accessory_writes: HashMap<SlotKey, Option<SlotValue>>,
}

impl<S: Storage> Delta<S> {
    #[cfg(feature = "native")]
    pub(super) fn clone_with_empty_witness(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            witness: Default::default(),
            kernel_cache: self.kernel_cache.clone(),
            user_cache: self.user_cache.clone(),
            accessory_writes: self.accessory_writes.clone(),
        }
    }

    #[cfg(feature = "native")]
    pub(super) fn inner(&self) -> &S {
        &self.inner
    }

    pub(super) fn with_witness(inner: S, witness: S::Witness) -> Self {
        Self {
            inner,
            witness,
            user_cache: Default::default(),
            kernel_cache: Default::default(),
            accessory_writes: Default::default(),
        }
    }

    pub(super) fn freeze(self) -> (StateAccesses, AccessoryDelta<S>, S::Witness, S) {
        let Self {
            inner,
            user_cache,
            kernel_cache,
            accessory_writes,
            witness,
        } = self;

        (
            StateAccesses {
                user: user_cache.to_ordered_writes_and_reads(),
                kernel: kernel_cache.to_ordered_writes_and_reads(),
            },
            AccessoryDelta {
                writes: accessory_writes,
                storage: inner.clone(),
            },
            witness,
            inner,
        )
    }

    pub(super) fn changes(&mut self) -> ChangeSet {
        self.commit_revertable_storage_cache();
        let changes = self
            .user_cache
            .get_writes()
            .map(|(k, v)| ((k.clone(), Namespace::User), v.cloned()))
            .chain(
                self.kernel_cache
                    .get_writes()
                    .map(|(k, v)| ((k.clone(), Namespace::Kernel), v.cloned())),
            )
            .chain(
                self.accessory_writes
                    .iter()
                    .map(|(k, v)| ((k.clone(), Namespace::Accessory), v.clone())),
            )
            .collect();
        ChangeSet { changes }
    }
}

impl<S: Storage> Delta<S> {
    pub fn commit_revertable_storage_cache(&mut self) {
        self.user_cache.commit_revertable_storage_cache();
        self.kernel_cache.commit_revertable_storage_cache();
    }

    pub fn discard_revertable_storage_cache(&mut self) {
        self.user_cache.discard_revertable_storage_cache();
        self.kernel_cache.discard_revertable_storage_cache();
    }

    pub fn is_value_cached(&self, namespace: Namespace, key: &SlotKey) -> IsValueCached {
        match namespace {
            Namespace::User => self.user_cache.is_value_cached(key),
            Namespace::Kernel => self.kernel_cache.is_value_cached(key),
            Namespace::Accessory => {
                if let Some(access) = self.accessory_writes.get(key) {
                    IsValueCached::Yes(AccessSize::Write(
                        access.as_ref().map(|v| v.size()).unwrap_or(0),
                    ))
                } else {
                    IsValueCached::No
                }
            }
        }
    }

    pub fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        match namespace {
            Namespace::User => self
                .user_cache
                .get_size_or_fetch(key, &self.inner, &self.witness),
            Namespace::Kernel => {
                self.kernel_cache
                    .get_size_or_fetch(key, &self.inner, &self.witness)
            }
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(Some(value)) => Some(value.size()),
                Some(None) => None,
                None => {
                    let val = self.inner.get_accessory(key, None);
                    val.map(|v| v.size())
                }
            },
        }
    }

    pub fn get(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        match namespace {
            Namespace::User => self
                .user_cache
                .get_or_fetch(key, &self.inner, &self.witness),
            Namespace::Kernel => self
                .kernel_cache
                .get_or_fetch(key, &self.inner, &self.witness),
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(Some(value)) => Some(value),
                Some(None) => None,
                None => self.inner.get_accessory(key, None),
            },
        }
    }

    pub fn set(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        match namespace {
            Namespace::User => self.user_cache.set(key, value),
            Namespace::Kernel => self.kernel_cache.set(key, value),
            Namespace::Accessory => {
                self.accessory_writes.insert(key.clone(), Some(value));
            }
        }
    }

    pub fn delete(&mut self, namespace: Namespace, key: &SlotKey) {
        match namespace {
            Namespace::User => self.user_cache.delete(key),
            Namespace::Kernel => self.kernel_cache.delete(key),
            Namespace::Accessory => {
                self.accessory_writes.remove(key);
            }
        }
    }
}

impl<S: Storage> fmt::Debug for Delta<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Delta").finish()
    }
}

/// A delta containing *only* the accessory state.
pub struct AccessoryDelta<S: Storage> {
    writes: HashMap<SlotKey, Option<SlotValue>>,
    storage: S,
}

impl<S: Storage> AccessoryDelta<S> {
    /// Freeze the accessory delta, preventing further accesses.
    pub fn freeze(self) -> Vec<(SlotKey, Option<SlotValue>)> {
        self.writes.into_iter().collect()
    }
}

impl<S: Storage> UniversalStateAccessor for AccessoryDelta<S> {
    fn get_size(&mut self, _namespace: Namespace, key: &SlotKey) -> Option<u32> {
        if let Some(value) = self.writes.get(key) {
            return value.clone().map(|v| v.size());
        }

        let val = self.storage.get_accessory(key, None);
        val.map(|v| v.size())
    }

    fn get_value(&mut self, _namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        if let Some(value) = self.writes.get(key) {
            return value.clone();
        }

        self.storage.get_accessory(key, None)
    }

    fn set_value(&mut self, _namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.writes.insert(key.clone(), Some(value));
    }

    fn delete_value(&mut self, _namespace: Namespace, key: &SlotKey) {
        self.writes.insert(key.clone(), None);
    }
}

pub(super) struct RevertableWriter<T> {
    pub(super) inner: T,
    writes: HashMap<(SlotKey, Namespace), Option<SlotValue>>,
    pub(crate) cache_writes: TempCache,
}

impl<T: fmt::Debug> fmt::Debug for RevertableWriter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevertableWriter")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<T> RevertableWriter<T> {
    pub(super) fn new(inner: T) -> Self {
        Self {
            inner,
            writes: HashMap::default(),
            cache_writes: TempCache::new(),
        }
    }

    /// Get an iterator over the current writes
    pub fn changes(&self) -> ChangeSet {
        ChangeSet::new(
            self.writes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }

    /// Commit all items from [`RevertableWriter`] returning the inner storage.
    pub(super) fn commit(mut self) -> T
    where
        T: UniversalStateAccessor + PerBlockCache,
    {
        for ((key, namespace), value) in self.writes {
            Self::commit_entry(&mut self.inner, namespace, &key, value);
        }

        self.inner.update_cache_with(self.cache_writes);

        self.inner
    }

    pub(super) fn revert(self) -> T {
        self.inner
    }

    fn commit_entry(inner: &mut T, namespace: Namespace, key: &SlotKey, value: Option<SlotValue>)
    where
        T: UniversalStateAccessor,
    {
        match value {
            Some(value) => inner.set_value(namespace, key, value),
            None => inner.delete_value(namespace, key),
        };
    }
}

impl<T> UniversalStateAccessor for RevertableWriter<T>
where
    T: UniversalStateAccessor,
{
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        if let Some(value) = self.writes.get(&(key.clone(), namespace)) {
            value.as_ref().map(|v| v.size())
        } else {
            <T as UniversalStateAccessor>::get_size(&mut self.inner, namespace, key)
        }
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        if let Some(value) = self.writes.get(&(key.clone(), namespace)) {
            value.clone()
        } else {
            <T as UniversalStateAccessor>::get_value(&mut self.inner, namespace, key)
        }
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.writes.insert((key.clone(), namespace), Some(value));
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        self.writes.insert((key.clone(), namespace), None);
    }
}

impl<C> RevertableWriter<C>
where
    C: PerBlockCache,
{
    pub(crate) fn get_cached<T: 'static + Send + Sync>(&self) -> Option<&T> {
        if let CacheLookup::Hit(value) = self.cache_writes.get::<T>() {
            value
        } else {
            self.inner.get_cached::<T>()
        }
    }
}
