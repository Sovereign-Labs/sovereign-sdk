use std::{
    collections::{HashMap, VecDeque},
    marker::PhantomData,
    sync::Arc,
};

#[cfg(feature = "native")]
use crate::{digest::typenum, Digest};

#[cfg(feature = "native")]
use crate::{ProvableNamespace, Namespace, OrderedReadsAndWrites, StateGetter};
use crate::{
    namespaces, AccessoryWrite, ProvableStorageCache, SlotKey, SlotValue, StateAccesses,
};

/// The list of state changes for a single rollup block.
#[derive(Debug, Clone, Default)]
pub struct RawStateChanges {
    /// changes to the user state
    pub user: ProvableStorageCache<namespaces::User>,
    /// changes to the kernel state
    pub kernel: ProvableStorageCache<namespaces::Kernel>,
    /// changes to the accessory state
    pub accessory: HashMap<SlotKey, AccessoryWrite>,
    /// The rollup height at which the changes were made.
    pub rollup_height: u64,
}

impl RawStateChanges {
    /// Convert the raw state changes to a [`StateAccesses`] instance for use in state root computation.
    /// Note that this excludes reads and does *not* sort the writes.
    #[cfg(feature = "native")]
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

#[cfg(feature = "native")]
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
