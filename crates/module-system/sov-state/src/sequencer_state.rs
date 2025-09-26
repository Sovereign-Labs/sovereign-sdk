use std::{
    collections::{HashMap, VecDeque},
    marker::PhantomData,
    sync::Arc,
};

#[cfg(feature = "native")]
use crate::NodeLeafAndMaybeValue;
use crate::{digest::typenum, internal::CacheLog, Digest, ProvableCompileTimeNamespace};

use crate::Access;
use crate::{namespaces, AccessoryWrite, ProvableStorageCache, SlotKey, SlotValue, StateAccesses};
use crate::{Namespace, OrderedReadsAndWrites, ProvableNamespace, StateGetter};

/// The list of state changes for a single rollup block.
#[derive(Debug, Clone, Default)]
pub struct RawStateChanges {
    /// changes to the user state
    pub user: CachedWrites<namespaces::User>,
    /// changes to the kernel state
    pub kernel: CachedWrites<namespaces::Kernel>,
    /// changes to the accessory state
    pub accessory: HashMap<SlotKey, AccessoryWrite>,
    /// The rollup height at which the changes were made.
    pub rollup_height: u64,
}

impl RawStateChanges {
    /// Convert the raw state changes to a [`StateAccesses`] instance for use in state root computation.
    /// Note that this excludes reads and does *not* sort the writes.
    pub fn to_state_accesses_for_sequencer_state_root_computation(&self) -> StateAccesses {
        let user_writes = self
            .user
            .get_writes()
            .map(|(k, v)| (k.clone(), v.cloned()))
            .collect();
        let kernel_writes = self
            .kernel
            .get_writes()
            .map(|(k, v)| (k.clone(), v.cloned()))
            .collect();
        StateAccesses {
            user: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: user_writes,
            },
            kernel: OrderedReadsAndWrites {
                ordered_reads: Vec::new(),
                ordered_writes: kernel_writes,
            },
        }
    }
}

/// A simplified analog of `ProvableStorageCache` for the sequencer state. This cache ignores any read values and only returns information
/// about writes.
#[derive(Default, Debug)]
pub struct CachedWrites<N> {
    // Transaction cache.
    cache: CacheLog,
    phantom: core::marker::PhantomData<N>,
}

impl<N: ProvableCompileTimeNamespace> Clone for CachedWrites<N> {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            phantom: self.phantom,
        }
    }
}

impl<N> From<ProvableStorageCache<N>> for CachedWrites<N> {
    fn from(cache: ProvableStorageCache<N>) -> Self {
        Self {
            cache: cache.cache,
            phantom: PhantomData,
        }
    }
}

#[cfg(feature = "test-utils")]
impl<N: ProvableCompileTimeNamespace> CachedWrites<N> {
    /// Set a value in the cache.
    pub fn set(&mut self, key: &SlotKey, value: SlotValue) {
        self.cache.add_write(key.clone(), Some(value));
    }

    /// Commit the revertable part of the cache.
    pub fn commit_revertable_storage_cache(&mut self) {
        self.cache.commit_revertable_log();
    }
}

impl<N: ProvableCompileTimeNamespace> CachedWrites<N> {
    /// Returns an iterator over the writes
    pub fn get_writes(&self) -> impl Iterator<Item = (&SlotKey, Option<&SlotValue>)> {
        self.cache.iter().filter_map(|(k, access)| {
            if let Access::Write { modified } = access {
                Some((k, modified.as_ref()))
            } else {
                None
            }
        })
    }

    /// Gets a value from the cache, if present
    pub fn get_from_cache(&self, key: &SlotKey) -> MaybePresentValue<SlotValue> {
        match self.cache.get(key) {
            Some(Access::Write { modified, .. }) => MaybePresentValue::Present(modified.clone()),
            // We don't want to return the values of old reads; we're only looking for values that were written by the block at the given height.
            Some(Access::Read { .. }) | None => MaybePresentValue::Absent,
        }
    }

    /// Gets a leaf from the cache, if present.
    pub fn get_leaf_from_cache<H: Digest<OutputSize = typenum::U32>>(
        &self,
        key: &SlotKey,
    ) -> MaybePresentValue<NodeLeafAndMaybeValue> {
        match self.cache.get(key) {
            // We don't want to return the values of old reads; we're only looking for values that were written by the block at the given height.
            Some(Access::Read { .. }) | None => MaybePresentValue::Absent,
            // Correctness: We only use the no-op hasher when the value is in intermediate state. This can happen in one of two cases:
            // - In the sequencer, where the value hash is unused
            // - During optimistic execution. If we executed optimistically, then this read will only have been in the intermediate state if it was previously written by an early transaction.
            // - In that case, the "read" will be discarded during the cache reconciliation procedure.
            Some(Access::Write { modified, .. }) => {
                use crate::{NodeLeaf, NodeLeafAndMaybeValue, ReadType};

                MaybePresentValue::Present(modified.as_ref().map(|v| NodeLeafAndMaybeValue {
                    leaf: NodeLeaf::make_leaf::<H>(v),
                    value: ReadType::Read(v.clone()),
                }))
            }
        }
    }
}

/// A collection of state changes from contiguous blocks, ordered by the rollup height at which they were made - newest first.
pub struct SequencerStateChanges<H> {
    /// The changes, ordered from newest to oldest
    pub changes: Option<VecDeque<Arc<RawStateChanges>>>,
    /// The hasher
    pub phantom: PhantomData<H>,
}

impl<H> Default for SequencerStateChanges<H> {
    fn default() -> Self {
        SequencerStateChanges {
            changes: None,
            phantom: PhantomData,
        }
    }
}

impl<H> Clone for SequencerStateChanges<H> {
    fn clone(&self) -> Self {
        SequencerStateChanges {
            changes: self.changes.clone(),
            phantom: PhantomData,
        }
    }
}

impl<H> std::fmt::Debug for SequencerStateChanges<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SequencerStateChanges<H>")
    }
}

impl<H> SequencerStateChanges<H> {
    /// Push a new set of changes to the front of the list.
    pub fn push_front(&mut self, changes: Arc<RawStateChanges>) {
        self.changes.get_or_insert_default().push_front(changes);
    }

    /// Prune all changes which took place up to and including the given height.
    pub fn prune_changes_through(&mut self, rollup_height: u64) {
        if let Some(changes) = self.changes.as_mut() {
            changes.retain(|change| change.rollup_height > rollup_height);
        }
    }

    /// Convert the sequencer state changes to a [`StateAccesses`] instance.
    pub fn to_state_accesses(&self) -> StateAccesses {
        let mut state_accesses = StateAccesses::default();
        for change in self.changes.iter().flatten() {
            state_accesses.user.ordered_writes.extend(
                change
                    .user
                    .get_writes()
                    .map(|(k, v)| (k.clone(), v.cloned())),
            );
            state_accesses.kernel.ordered_writes.extend(
                change
                    .kernel
                    .get_writes()
                    .map(|(k, v)| (k.clone(), v.cloned())),
            );
        }
        // Ensure that only the latest write for each key is reflected by...
        // - Sorting (stably) by key. This ensures that identical keys are next to each other and the newest write is first
        // = Use dedup by to remove all but the first instance of each key
        state_accesses
            .user
            .ordered_writes
            .sort_by_key(|(k, _v)| k.clone());
        state_accesses
            .user
            .ordered_writes
            .dedup_by(|(k1, _v1), (k2, _v2)| k1 == k2);
        // Do the same for the kernel namespace
        state_accesses
            .kernel
            .ordered_writes
            .sort_by_key(|(k, _v)| k.clone());
        state_accesses
            .kernel
            .ordered_writes
            .dedup_by(|(k1, _v1), (k2, _v2)| k1 == k2); // Sort stably

        state_accesses
    }
}

/// A value that may not be present in the given cache.
pub enum MaybePresentValue<T = SlotValue> {
    /// The key is present in the cache; it's value may be some or none
    Present(Option<T>),
    /// The key is absent from the cache.
    Absent,
}

impl<T> MaybePresentValue<T> {
    /// Map the value if it is present.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> MaybePresentValue<U> {
        match self {
            MaybePresentValue::Present(value) => MaybePresentValue::Present(value.map(f)),
            MaybePresentValue::Absent => MaybePresentValue::Absent,
        }
    }

    /// Get the value if it's present, calling the provided function otherwise.
    pub fn or_else<F: FnOnce() -> Option<T>>(self, f: F) -> Option<T> {
        match self {
            MaybePresentValue::Present(value) => value,
            MaybePresentValue::Absent => f(),
        }
    }
}

impl<H: Digest<OutputSize = typenum::U32> + Send + Sync + 'static> StateGetter
    for SequencerStateChanges<H>
{
    fn get(&self, namespace: Namespace, key: &SlotKey) -> MaybePresentValue {
        for change_set in self.changes.iter().flatten() {
            if let MaybePresentValue::Present(maybe_value) = match namespace {
                Namespace::User => change_set.user.get_from_cache(key),
                Namespace::Kernel => change_set.kernel.get_from_cache(key),
                Namespace::Accessory => match change_set
                    .accessory
                    .get(key)
                    .map(|write| write.value.clone())
                {
                    Some(maybe_value) => MaybePresentValue::Present(maybe_value),
                    None => MaybePresentValue::Absent,
                },
            } {
                return MaybePresentValue::Present(maybe_value);
            }
        }
        MaybePresentValue::Absent
    }

    fn get_leaf(
        &self,
        namespace: ProvableNamespace,
        key: &SlotKey,
    ) -> MaybePresentValue<crate::NodeLeafAndMaybeValue> {
        for change_set in self.changes.iter().flatten() {
            if let MaybePresentValue::Present(maybe_value) = match namespace {
                ProvableNamespace::User => change_set.user.get_leaf_from_cache::<H>(key),
                ProvableNamespace::Kernel => change_set.kernel.get_leaf_from_cache::<H>(key),
            } {
                return MaybePresentValue::Present(maybe_value);
            }
        }
        MaybePresentValue::Absent
    }

    fn box_clone(&self) -> Box<dyn StateGetter> {
        Box::new(self.clone())
    }
}
