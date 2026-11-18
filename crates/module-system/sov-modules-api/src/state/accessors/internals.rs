use core::fmt;
use std::collections::HashMap;

use crate::state::accessors::StateMetricsProvider;
use crate::{Spec, StateCheckpoint, TxChangeSet};
use sov_metrics::{StateAccessMetric, StateMetrics};
use sov_rollup_interface::stf::ExecutionContext;
pub(crate) use sov_state::AccessoryWrite;
#[cfg(feature = "native")]
use sov_state::StateGetter;
use sov_state::{
    namespaces, AccessSize, IsValueCached, Namespace, ProvableStorageCache, SlotKey, SlotValue,
    StateAccesses, Storage,
};
use sov_state::{NodeLeafAndMaybeValue, DEFAULT_CACHE_CAPACITY};

#[cfg(feature = "native")]
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
    #[cfg(feature = "native")]
    // Changes that are not yet committed to the underlying storage that should be taken into account when querying the storage
    pub(crate) uncommitted_changes: Option<Box<dyn StateGetter>>,
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
            uncommitted_changes: self.uncommitted_changes.as_ref().map(|g| g.box_clone()),
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
            uncommitted_changes: None,
            user_cache: Default::default(),
            kernel_cache: Default::default(),
            accessory_writes: HashMap::with_capacity(DEFAULT_CACHE_CAPACITY),
        }
    }

    #[cfg(feature = "native")]
    pub(super) fn take_accessory_delta(&mut self) -> AccessoryDelta<S> {
        AccessoryDelta {
            writes: std::mem::replace(
                &mut self.accessory_writes,
                HashMap::with_capacity(DEFAULT_CACHE_CAPACITY),
            ),
            #[cfg(feature = "native")]
            uncommitted_changes: self.uncommitted_changes.as_ref().map(|g| g.box_clone()),
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
            uncommitted_changes,
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
                uncommitted_changes,
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
    pub user: Vec<(SlotKey, Option<NodeLeafAndMaybeValue>)>,
    /// Kernel space reads.
    pub kernel: Vec<(SlotKey, Option<NodeLeafAndMaybeValue>)>,
}

impl<S: Storage> Delta<S> {
    pub fn first_reads(&self) -> FirstTimeReads {
        FirstTimeReads {
            user: self.user_cache.revertable_ordered_reads().clone(),
            kernel: self.kernel_cache.revertable_ordered_reads().clone(),
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
                &self.uncommitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Kernel => self.kernel_cache.get_size_or_fetch(
                &self.uncommitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(write) => write.value.as_ref().map(|v| v.size()),
                None => {
                    let val = match self.uncommitted_changes.as_ref() {
                        Some(uncommitted_changes) => uncommitted_changes
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

    /// See [`UniversalStateAccessor::get_cached_value`].
    pub fn get_cached_value(&self, namespace: Namespace, key: &SlotKey) -> Option<Option<SlotValue>> {
        match namespace {
            Namespace::User => self.user_cache.get_cached_value(key),
            Namespace::Kernel => self.kernel_cache.get_cached_value(key),
            Namespace::Accessory => None,
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
                &self.uncommitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Kernel => self.kernel_cache.get_or_fetch(
                &self.uncommitted_changes,
                key,
                &self.inner,
                &self.witness,
                metric,
            ),
            Namespace::Accessory => match self.accessory_writes.get(key).cloned() {
                Some(write) => write.value,
                None => {
                    let val = if let Some(uncommitted_changes) = self.uncommitted_changes.as_ref() {
                        return uncommitted_changes
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

    pub fn add_read_if_not_present(&mut self, reads: FirstTimeReads) {
        self.user_cache.add_read_if_not_present_in_cache(reads.user);
        self.kernel_cache
            .add_read_if_not_present_in_cache(reads.kernel);
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
    uncommitted_changes: Option<Box<dyn StateGetter>>,
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

        let val = if let Some(uncommitted_changes) = self.uncommitted_changes.as_ref() {
            uncommitted_changes
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

        let val = if let Some(uncommitted_changes) = self.uncommitted_changes.as_ref() {
            uncommitted_changes
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
    pub(crate) writes: PendingWrites,
    pub(crate) cache_writes: TempCache,
}

/// Pending writes of a [`RevertableWriter`]: one map per namespace, so a lookup hashes the
/// borrowed key instead of building a `(key, namespace)` tuple.
pub struct PendingWrites {
    user: HashMap<SlotKey, Option<SlotValue>>,
    kernel: HashMap<SlotKey, Option<SlotValue>>,
    accessory: HashMap<SlotKey, Option<SlotValue>>,
}

fn merge_newer_map(
    older: &mut HashMap<SlotKey, Option<SlotValue>>,
    mut newer: HashMap<SlotKey, Option<SlotValue>>,
) {
    if newer.len() > older.len() {
        // Keep the bigger map and add the older entries it doesn't override.
        std::mem::swap(older, &mut newer);
        for (key, value) in newer {
            older.entry(key).or_insert(value);
        }
    } else {
        older.extend(newer);
    }
}

impl PendingWrites {
    fn new() -> Self {
        Self {
            // Transactions write a few keys per layer; allocate on the first write.
            user: HashMap::new(),
            kernel: HashMap::new(),
            accessory: HashMap::new(),
        }
    }

    fn map(&self, namespace: Namespace) -> &HashMap<SlotKey, Option<SlotValue>> {
        match namespace {
            Namespace::User => &self.user,
            Namespace::Kernel => &self.kernel,
            Namespace::Accessory => &self.accessory,
        }
    }

    fn map_mut(&mut self, namespace: Namespace) -> &mut HashMap<SlotKey, Option<SlotValue>> {
        match namespace {
            Namespace::User => &mut self.user,
            Namespace::Kernel => &mut self.kernel,
            Namespace::Accessory => &mut self.accessory,
        }
    }

    pub(crate) fn get(&self, namespace: Namespace, key: &SlotKey) -> Option<&Option<SlotValue>> {
        self.map(namespace).get(key)
    }

    fn insert(&mut self, namespace: Namespace, key: &SlotKey, value: Option<SlotValue>) {
        // Cloning the key is cheaper than hashing it twice to check for an existing entry first.
        self.map_mut(namespace).insert(key.clone(), value);
    }

    fn iter(&self) -> impl Iterator<Item = ((&SlotKey, Namespace), &Option<SlotValue>)> {
        [
            (Namespace::User, &self.user),
            (Namespace::Kernel, &self.kernel),
            (Namespace::Accessory, &self.accessory),
        ]
        .into_iter()
        .flat_map(|(namespace, map)| map.iter().map(move |(k, v)| ((k, namespace), v)))
    }

    /// Adds `newer` writes on top of these ones, as inserting each of them would, but hashes only
    /// the entries of whichever map is smaller.
    pub(crate) fn merge_newer(&mut self, newer: PendingWrites) {
        let PendingWrites {
            user,
            kernel,
            accessory,
        } = newer;
        merge_newer_map(&mut self.user, user);
        merge_newer_map(&mut self.kernel, kernel);
        merge_newer_map(&mut self.accessory, accessory);
    }

    pub(crate) fn into_iter(self) -> impl Iterator<Item = ((SlotKey, Namespace), Option<SlotValue>)> {
        [
            (Namespace::User, self.user),
            (Namespace::Kernel, self.kernel),
            (Namespace::Accessory, self.accessory),
        ]
        .into_iter()
        .flat_map(|(namespace, map)| map.into_iter().map(move |(k, v)| ((k, namespace), v)))
    }
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
            writes: PendingWrites::new(),
            cache_writes: TempCache::new(),
        }
    }

    /// Commit all items from [`RevertableWriter`] returning the inner storage.
    pub(super) fn commit(mut self) -> T
    where
        T: UniversalStateAccessor + PerBlockCache,
    {
        self.inner.commit_writes(self.writes);

        self.inner.update_cache_with(self.cache_writes);

        self.inner
    }

    pub(super) fn revert(self) -> T {
        self.inner
    }

}

impl<S: Spec> RevertableWriter<StateCheckpoint<S>> {
    /// Change set resulting from transaction execution.
    pub fn changes(&self, execution_context: ExecutionContext) -> TxChangeSet {
        // Only the WarmUp executors warm up the reads.
        let reads = if matches!(execution_context, ExecutionContext::SequencerWarmUp) {
            Some(self.inner.first_reads().clone())
        } else {
            None
        };

        TxChangeSet {
            writes: self
                .writes
                .iter()
                .map(|((k, namespace), v)| ((k.clone(), namespace), v.clone()))
                .collect(),
            reads,
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
        if let Some(value) = self.writes.get(namespace, key) {
            value.as_ref().map(|v| v.size())
        } else {
            <T as UniversalStateAccessor>::get_size(&mut self.inner, namespace, key, metric)
        }
    }

    fn get_cached_value(&self, namespace: Namespace, key: &SlotKey) -> Option<Option<SlotValue>> {
        match self.writes.get(namespace, key) {
            Some(value) => Some(value.clone()),
            None => self.inner.get_cached_value(namespace, key),
        }
    }

    fn get_value(
        &mut self,
        namespace: Namespace,
        key: &SlotKey,
        metric: &mut StateAccessMetric,
    ) -> Option<SlotValue> {
        if let Some(value) = self.writes.get(namespace, key) {
            value.clone()
        } else {
            <T as UniversalStateAccessor>::get_value(&mut self.inner, namespace, key, metric)
        }
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.writes.insert(namespace, key, Some(value));
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        self.writes.insert(namespace, key, None);
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
