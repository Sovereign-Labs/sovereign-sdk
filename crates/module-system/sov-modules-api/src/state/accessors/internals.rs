use core::fmt;
use std::collections::HashMap;

use crate::state::accessors::StateMetricsProvider;
use crate::{Spec, StateCheckpoint, TxChangeSet};
use sov_metrics::{StateAccessMetric, StateMetrics};
pub(crate) use sov_state::AccessoryWrite;
#[cfg(feature = "native")]
use sov_state::StateGetter;
use sov_state::{
    namespaces, AccessSize, IsValueCached, Namespace, ProvableStorageCache, SlotKey, SlotValue,
    StateAccesses, Storage,
};

#[cfg(feature = "native")]
use super::checkpoints::ChangeSet;
use super::temp_cache::{CacheLookup, TempCache};
use super::UniversalStateAccessor;
use crate::state::traits::PerBlockCache;
use sov_state::NodeLeaf;

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
    #[cfg(feature = "native")]
    // Changes that are not yet committed to the underlying storage that should be taken into account when querying the storage
    pub(crate) uncomitted_changes: Option<Box<dyn StateGetter>>,
    pub(crate) kernel_cache: ProvableStorageCache<namespaces::Kernel>,
    pub(crate) user_cache: ProvableStorageCache<namespaces::User>,
    pub(crate) accessory_writes: HashMap<SlotKey, AccessoryWrite>,
}

impl<S: Storage> Delta<S> {
    #[cfg(feature = "native")]
    pub(super) fn clone_with_empty_witness(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            witness: Default::default(),
            uncomitted_changes: self.uncomitted_changes.as_ref().map(|g| g.box_clone()),
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
            #[cfg(feature = "native")]
            uncomitted_changes: None,
            user_cache: Default::default(),
            kernel_cache: Default::default(),
            accessory_writes: Default::default(),
        }
    }

    #[cfg(feature = "native")]
    pub(super) fn take_accessory_delta(&mut self) -> AccessoryDelta<S> {
        AccessoryDelta {
            writes: std::mem::take(&mut self.accessory_writes),
            #[cfg(feature = "native")]
            uncomitted_changes: self.uncomitted_changes.as_ref().map(|g| g.box_clone()),
            storage: self.inner.clone(),
            metrics: StateMetrics::default(),
        }
    }

    #[cfg(feature = "native")]
    pub(super) fn set_accessory_delta(&mut self, accessory_delta: AccessoryDelta<S>) {
        self.accessory_writes = accessory_delta.writes;
    }

    pub(super) fn freeze(self) -> (StateAccesses, AccessoryDelta<S>, S::Witness, S) {
        let Self {
            inner,
            user_cache,
            kernel_cache,
            accessory_writes,
            witness,
            #[cfg(feature = "native")]
            uncomitted_changes,
        } = self;

        (
            StateAccesses {
                user: user_cache.to_ordered_writes_and_reads(),
                kernel: kernel_cache.to_ordered_writes_and_reads(),
            },
            AccessoryDelta {
                writes: accessory_writes,
                storage: inner.clone(),
                #[cfg(feature = "native")]
                uncomitted_changes,
                metrics: StateMetrics::default(),
            },
            witness,
            inner,
        )
    }

    #[cfg(feature = "native")]
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

/// Holds keys and values that were read for the first time.
#[derive(Debug, Clone, Default)]
pub struct FirstTimeReads {
    /// User space reads.
    pub user_reads: Vec<(SlotKey, Option<NodeLeaf>)>,
    /// Kernel space reads.
    pub kernel_reads: Vec<(SlotKey, Option<NodeLeaf>)>,
}

impl<S: Storage> Delta<S> {
    pub fn first_reads(&self) -> FirstTimeReads {
        FirstTimeReads {
            user_reads: self.user_cache.revertable_ordered_reads().clone(),
            kernel_reads: self.kernel_cache.revertable_ordered_reads().clone(),
        }
    }

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

    #[cfg(feature = "native")]
    pub fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<u32> {
        match namespace {
            Namespace::User => self.user_cache.get_size_or_fetch(
                &self.uncomitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Kernel => self.kernel_cache.get_size_or_fetch(
                &self.uncomitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(write) => write.value.as_ref().map(|v| v.size()),
                None => {
                    let val = match self.uncomitted_changes.as_ref() {
                        Some(uncomitted_changes) => uncomitted_changes
                            .get(Namespace::Accessory, key)
                            .or_else(|| self.inner.get_accessory(key)),
                        None => self.inner.get_accessory(key),
                    };
                    let size = val.map(|v| v.size());
                    metric.storage_read_size = Some(size.unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
                    size
                }
            },
        }
    }

    #[cfg(not(feature = "native"))]
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

    #[cfg(feature = "native")]
    pub fn get(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        match namespace {
            Namespace::User => self.user_cache.get_or_fetch(
                &self.uncomitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Kernel => self.kernel_cache.get_or_fetch(
                &self.uncomitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(write) => write.value,
                None => {
                    let val = if let Some(uncomitted_changes) = self.uncomitted_changes.as_ref() {
                        return uncomitted_changes
                            .get(namespace, key)
                            .or_else(|| self.inner.get_accessory(key));
                    } else {
                        self.inner.get_accessory(key)
                    };
                    let size = val.as_ref().map(|v| v.size());
                    metric.storage_read_size = Some(size.unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
                    val
                }
            },
        }
    }

    #[cfg(not(feature = "native"))]
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

    pub fn set(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        match namespace {
            Namespace::User => self.user_cache.set(key, value),
            Namespace::Kernel => self.kernel_cache.set(key, value),
            Namespace::Accessory => {
                self.accessory_writes
                    .insert(key.clone(), AccessoryWrite::new(Some(value)));
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
    writes: HashMap<SlotKey, AccessoryWrite>,
    #[cfg(feature = "native")]
    uncomitted_changes: Option<Box<dyn StateGetter>>,
    storage: S,
    metrics: StateMetrics,
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
    #[cfg(not(feature = "native"))]
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

    #[cfg(not(feature = "native"))]
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

    #[cfg(feature = "native")]
    fn get_size(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<u32> {
        if let Some(write) = self.writes.get(key) {
            return write.value.as_ref().map(|v| v.size());
        }

        let val = if let Some(uncomitted_changes) = self.uncomitted_changes.as_ref() {
            uncomitted_changes
                .get(namespace, key)
                .or_else(|| self.storage.get_accessory(key))
        } else {
            self.storage.get_accessory(key)
        };

        metric.storage_read_size = Some(val.as_ref().map(|v| v.size()).unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
        val.map(|v| v.size())
    }

    #[cfg(feature = "native")]
    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        if let Some(write) = self.writes.get(key) {
            return write.value.clone();
        }

        let val = if let Some(uncomitted_changes) = self.uncomitted_changes.as_ref() {
            uncomitted_changes
                .get(namespace, key)
                .or_else(|| self.storage.get_accessory(key))
        } else {
            self.storage.get_accessory(key)
        };
        metric.storage_read_size = Some(val.as_ref().map(|v| v.size()).unwrap_or(0)); // For the metric, use "Some" to indicate that we hit storage even if the value is None
        val
    }

    fn set_value(&mut self, _namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.writes
            .insert(key.clone(), AccessoryWrite::new(Some(value)));
    }

    fn delete_value(&mut self, _namespace: Namespace, key: &SlotKey) {
        self.writes.insert(key.clone(), AccessoryWrite::new(None));
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

impl<S: Spec> RevertableWriter<StateCheckpoint<S>> {
    /// Change set resulting from transaction execution.
    pub fn changes(&self) -> TxChangeSet {
        TxChangeSet {
            writes: self
                .writes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            reads: self.inner.first_reads().clone(),
        }
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
