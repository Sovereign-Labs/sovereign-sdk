use std::sync::{Arc, PoisonError, RwLock};

use sov_rollup_interface::common::{RollupHeight, SlotNumber, VisibleSlotNumber};
use sov_state::{Namespace, NativeStorage, SlotKey, SlotValue, StateGetter};

use crate::{EventEpoch, Spec, StateCheckpoint, TxChangeSet};

/// A read transaction over a [`ConcurrentStateCheckpoint`]'s pending writes.
type WritesReadTxn<'a> =
    concread::hashmap::HashMapReadTxn<'a, (SlotKey, Namespace), Option<SlotValue>>;

/// How far event numbering has progressed, and in which run of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFrontier {
    /// The run of numbering that [`Self::next_event_number`] belongs to.
    pub epoch: EventEpoch,
    /// One past the last event whose effects are reflected in the associated state.
    pub next_event_number: u64,
}

impl EventFrontier {
    #[allow(missing_docs)]
    pub fn new(epoch: EventEpoch, next_event_number: u64) -> Self {
        Self {
            epoch,
            next_event_number,
        }
    }

    /// The last event reflected, or `None` before the first event is emitted.
    pub fn last_event_number(&self) -> Option<u64> {
        self.next_event_number.checked_sub(1)
    }
}

/// How `finalized` RPC reads should be resolved for this checkpoint.
///
/// This policy has to be explicit because the finalized slot number alone is
/// not enough to infer semantics:
/// - In preferred sequencer mode, finalized intentionally follows the storage head.
/// - In standard sequencer mode, finalized is the node-reported finalized slot.
///
/// Both modes can have the same numeric slot at a given instant, so callers
/// cannot safely infer the intended behavior from slot equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizedSlotPolicy {
    /// `finalized` should track whatever slot is currently at the storage head.
    TrackStorageHead,
    /// `finalized` should use an explicit slot provided by the node.
    UseExplicit(SlotNumber),
}

/// An analogue of `StateCheckpoint` that can be safely written while concurrent reads are happening.
pub struct ConcurrentStateCheckpoint<S: Spec> {
    pub(super) storage: S::Storage,
    pub(crate) uncommitted_changes: Option<Box<dyn StateGetter>>,
    /// # Invariant
    ///
    /// Every `writes.write()`/`commit()` performed after this checkpoint has been published
    /// (i.e. anything but the construction below) MUST happen while holding the
    /// `next_event_number` *write* guard. That is what makes the pairing in
    /// [`Self::event_frontier_with_read_txn`] exact; see [`Self::apply_tx_changes`].
    pub(crate) writes: Arc<concread::hashmap::HashMap<(SlotKey, Namespace), Option<SlotValue>>>,
    /// How far event numbering has progressed in `writes`. `None` when the producer does not
    /// track event numbers.
    ///
    /// Readers must acquire this lock *before* taking a `writes` read transaction and hold it
    /// across the acquisition, so that the frontier and the state snapshot describe the same
    /// instant. Lock order is `event_frontier` -> `writes`.
    event_frontier: RwLock<Option<EventFrontier>>,
    pub(super) visible_slot_num: VisibleSlotNumber,
    pub(super) rollup_height: RollupHeight,
    pub(super) finalized_slot_policy: FinalizedSlotPolicy,
}

impl<S: Spec> ConcurrentStateCheckpoint<S> {
    /// Create a `ConcurrentStateCheckpoint` containing the same changes as the given `StateCheckpoint`.
    ///
    /// Note: this defaults the latest finalized slot to the most recent slot
    /// available in storage. That effectively treats all known slots as
    /// finalized. Call `from_state_checkpoint_with_finalized_slot` if you need
    /// to preserve the node's true finalized slot semantics.
    pub fn from_state_checkpoint(state_checkpoint: StateCheckpoint<S>) -> Self {
        let latest_finalized_slot_number = state_checkpoint.delta.inner.latest_version();
        let mut checkpoint = Self::from_state_checkpoint_with_finalized_slot(
            state_checkpoint,
            latest_finalized_slot_number,
        );
        checkpoint.finalized_slot_policy = FinalizedSlotPolicy::TrackStorageHead;
        checkpoint
    }

    /// Create a `ConcurrentStateCheckpoint` containing the same changes as the given `StateCheckpoint`,
    /// and with an explicit latest finalized slot number.
    pub fn from_state_checkpoint_with_finalized_slot(
        mut state_checkpoint: StateCheckpoint<S>,
        latest_finalized_slot_number: SlotNumber,
    ) -> Self {
        let map = concread::hashmap::HashMap::new();
        state_checkpoint.delta.commit_revertable_storage_cache();
        let mut writer = map.write();
        for (key, value) in state_checkpoint.delta.user_cache.take_writes() {
            writer.insert((key.clone(), Namespace::User), value);
        }
        for (key, value) in state_checkpoint.delta.kernel_cache.take_writes() {
            writer.insert((key.clone(), Namespace::Kernel), value);
        }
        for (key, value) in state_checkpoint.delta.accessory_writes.into_iter() {
            writer.insert((key.clone(), Namespace::Accessory), value.value);
        }
        writer.commit();

        let max_available_slot = state_checkpoint.delta.inner.latest_version();
        let latest_finalized_slot_number = latest_finalized_slot_number.min(max_available_slot);

        Self {
            storage: state_checkpoint.delta.inner,
            uncommitted_changes: state_checkpoint.delta.uncommitted_changes,
            writes: Arc::new(map),
            event_frontier: RwLock::new(None),
            visible_slot_num: state_checkpoint.visible_slot_num,
            rollup_height: state_checkpoint.rollup_height,
            finalized_slot_policy: FinalizedSlotPolicy::UseExplicit(latest_finalized_slot_number),
        }
    }

    /// Records how far event numbering has progressed in this checkpoint.
    ///
    /// Call this on a freshly built checkpoint, before publishing it, to state which events
    /// its contents already reflect. Checkpoints that are never published, or whose producer
    /// does not track event numbers, can leave it unset.
    pub fn with_event_frontier(self, event_frontier: EventFrontier) -> Self {
        *self
            .event_frontier
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(event_frontier);
        self
    }

    /// How far event numbering has progressed *right now*.
    ///
    /// Use this to decide what to publish next: a new frontier below this one means the
    /// numbering rewound and must be given a fresh [`EventEpoch`].
    pub fn event_frontier(&self) -> Option<EventFrontier> {
        *self
            .event_frontier
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns the event frontier paired with a read transaction over `writes`.
    ///
    /// The returned frontier is *exact* for the returned snapshot: the snapshot reflects the
    /// effects of every event below `next_event_number`, and of no event at or above it. See
    /// the lock-ordering note on [`Self::apply_tx_changes`].
    pub(crate) fn event_frontier_with_read_txn(
        &self,
    ) -> (Option<EventFrontier>, WritesReadTxn<'_>) {
        let guard = self
            .event_frontier
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let read_txn = self.writes.read();
        let event_frontier = *guard;
        drop(guard);
        (event_frontier, read_txn)
    }

    /// Apply the given `TxChangeSet` to the `ConcurrentStateCheckpoint`, advancing the event
    /// frontier to `new_next_event_number` if the transaction emitted events.
    ///
    /// This only ever moves the frontier forward within the current [`EventEpoch`]; rewinding
    /// the numbering is done by publishing a new checkpoint, not from here.
    ///
    /// The frontier write guard is taken *before* the `writes` write guard and released
    /// *after* the commit, so a concurrent reader can never observe the committed writes
    /// paired with the stale frontier (or vice versa). Do not reorder these acquisitions: the
    /// global lock order is `event_frontier` -> `writes`.
    pub fn apply_tx_changes(&self, changeset: TxChangeSet, new_next_event_number: Option<u64>) {
        let mut frontier = self
            .event_frontier
            .write()
            .unwrap_or_else(PoisonError::into_inner);

        let mut writer = self.writes.write();
        writer.extend(changeset.writes.into_iter().map(|(k, v)| (k, v.clone())));

        writer.commit();

        // A transaction that emitted no events leaves the frontier where it was: the state it
        // wrote is not attributable to any event, and no event beyond the frontier is now
        // reflected, so the pairing stays exact.
        if let Some(next_event_number) = new_next_event_number {
            if let Some(frontier) = frontier.as_mut() {
                frontier.next_event_number = next_event_number;
            }
        }
    }

    /// Get a reference to the underlying storage.
    pub fn storage(&self) -> &S::Storage {
        &self.storage
    }

    /// Get the visible slot number.
    pub fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.visible_slot_num
    }

    /// Get the rollup height to access.
    pub fn rollup_height_to_access(&self) -> RollupHeight {
        self.rollup_height
    }

    /// Get the latest finalized slot number available to this checkpoint.
    pub fn latest_finalized_slot_number(&self) -> SlotNumber {
        match self.finalized_slot_policy {
            FinalizedSlotPolicy::TrackStorageHead => self.storage.latest_version(),
            FinalizedSlotPolicy::UseExplicit(slot) => slot,
        }
    }

    /// Returns how `finalized` should be resolved for reads on this checkpoint.
    pub fn finalized_slot_policy(&self) -> FinalizedSlotPolicy {
        self.finalized_slot_policy
    }
}
