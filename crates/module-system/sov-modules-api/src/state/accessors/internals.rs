use core::fmt;
use std::collections::HashMap;

use crate::state::accessors::StateMetricsProvider;
use sov_metrics::{StateAccessMetric, StateMetrics};
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
    pub(crate) accessory_writes: HashMap<SlotKey, AccessoryWrite>,
}

#[derive(Debug, Clone)]
pub(crate) struct AccessoryWrite {
    #[cfg(feature = "native")]
    pub at_rollup_height: u64,
    pub value: Option<SlotValue>,
}

impl AccessoryWrite {
    #[cfg(feature = "native")]
    pub fn new(at_rollup_height: u64, value: Option<SlotValue>) -> Self {
        Self {
            at_rollup_height,
            value,
        }
    }

    #[cfg(not(feature = "native"))]
    pub fn new(_at_rollup_height: u64, value: Option<SlotValue>) -> Self {
        Self { value }
    }
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

    #[cfg(feature = "native")]
    pub(super) fn replace_storage_and_prune(&mut self, storage: S, rollup_height: u64) {
        self.inner = storage;
        self.user_cache
            .prune_writes_up_to_and_all_reads(rollup_height);
        self.kernel_cache
            .prune_writes_up_to_and_all_reads(rollup_height);
        self.accessory_writes
            .retain(|_, write| write.at_rollup_height >= rollup_height);
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

    pub(super) fn freeze(
        self,
        rollup_height: u64,
    ) -> (StateAccesses, AccessoryDelta<S>, S::Witness, S) {
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
                metrics: StateMetrics::default(),
                rollup_height,
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
                    .map(|(k, w)| ((k.clone(), Namespace::Accessory), w.value.clone())),
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
                        access.value.as_ref().map(|v| v.size()).unwrap_or(0),
                    ))
                } else {
                    IsValueCached::No
                }
            }
        }
    }

    pub fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<u32> {
        match namespace {
            Namespace::User => {
                self.user_cache
                    .get_size_or_fetch(key, &self.inner, &self.witness, metric)
            }
            Namespace::Kernel => {
                self.kernel_cache
                    .get_size_or_fetch(key, &self.inner, &self.witness, metric)
            }
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(write) => write.value.as_ref().map(|v| v.size()),
                None => {
                    let val = self.inner.get_accessory(key);
                    let size = val.map(|v| v.size());
                    metric.storage_read_size = Some(size.unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
                    size
                }
            },
        }
    }

    pub fn get(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        match namespace {
            Namespace::User => {
                self.user_cache
                    .get_or_fetch(key, &self.inner, &self.witness, metric)
            }
            Namespace::Kernel => {
                self.kernel_cache
                    .get_or_fetch(key, &self.inner, &self.witness, metric)
            }
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(write) => write.value,
                None => {
                    let val = self.inner.get_accessory(key);
                    let size = val.as_ref().map(|v| v.size());
                    metric.storage_read_size = Some(size.unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
                    val
                }
            },
        }
    }

    pub fn set(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        value: SlotValue,
        rollup_height: u64,
    ) {
        match namespace {
            Namespace::User => self.user_cache.set(key, value, rollup_height),
            Namespace::Kernel => self.kernel_cache.set(key, value, rollup_height),
            Namespace::Accessory => {
                self.accessory_writes
                    .insert(key.clone(), AccessoryWrite::new(rollup_height, Some(value)));
            }
        }
    }

    pub fn delete(&mut self, namespace: Namespace, key: &SlotKey, rollup_height: u64) {
        match namespace {
            Namespace::User => self.user_cache.delete(key, rollup_height),
            Namespace::Kernel => self.kernel_cache.delete(key, rollup_height),
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
    writes: HashMap<SlotKey, AccessoryWrite>,
    storage: S,
    metrics: StateMetrics,
    rollup_height: u64,
}

impl<S: Storage> StateMetricsProvider for AccessoryDelta<S> {
    fn metrics(&mut self) -> &mut StateMetrics {
        &mut self.metrics
    }
}

impl<S: Storage> AccessoryDelta<S> {
    /// Freeze the accessory delta, preventing further accesses.
    pub fn freeze(self) -> Vec<(SlotKey, Option<SlotValue>)> {
        self.writes.into_iter().map(|(k, v)| (k, v.value)).collect()
    }
}

impl<S: Storage> UniversalStateAccessor for AccessoryDelta<S> {
    fn get_size(
        &mut self,
        _namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<u32> {
        if let Some(write) = self.writes.get(key) {
            return write.value.as_ref().map(|v| v.size());
        }

        let val = self.storage.get_accessory(key);
        metric.storage_read_size = Some(val.as_ref().map(|v| v.size()).unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
        val.map(|v| v.size())
    }

    fn get_value(
        &mut self,
        _namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        if let Some(write) = self.writes.get(key) {
            return write.value.clone();
        }

        let val = self.storage.get_accessory(key);
        metric.storage_read_size = Some(val.as_ref().map(|v| v.size()).unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
        val
    }

    fn set_value(&mut self, _namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.writes.insert(
            key.clone(),
            AccessoryWrite::new(self.rollup_height, Some(value)),
        );
    }

    fn delete_value(&mut self, _namespace: Namespace, key: &SlotKey) {
        self.writes
            .insert(key.clone(), AccessoryWrite::new(self.rollup_height, None));
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
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<u32> {
        if let Some(value) = self.writes.get(&(key.clone(), namespace)) {
            value.as_ref().map(|v| v.size())
        } else {
            <T as UniversalStateAccessor>::get_size(&mut self.inner, namespace, key, metric)
        }
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        if let Some(value) = self.writes.get(&(key.clone(), namespace)) {
            value.clone()
        } else {
            <T as UniversalStateAccessor>::get_value(&mut self.inner, namespace, key, metric)
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
    pub(crate) fn get_cached<T: 'static + Send + Sync>(
        &self,
        slot_key: Option<SlotKey>,
    ) -> Option<&T> {
        if let CacheLookup::Hit(value) = self.cache_writes.get::<T>(slot_key.clone()) {
            value
        } else {
            self.inner.get_cached::<T>(slot_key)
        }
    }
}

impl<C: StateMetricsProvider> StateMetricsProvider for RevertableWriter<C> {
    fn metrics(&mut self) -> &mut StateMetrics {
        self.inner.metrics()
    }
}
