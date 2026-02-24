use std::sync::Arc;

use sov_rollup_interface::common::{RollupHeight, SlotNumber, VisibleSlotNumber};
use sov_state::{Namespace, NativeStorage, SlotKey, SlotValue, StateGetter};

use crate::{Spec, StateCheckpoint, TxChangeSet};

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
    pub(crate) writes: Arc<concread::hashmap::HashMap<(SlotKey, Namespace), Option<SlotValue>>>,
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
            visible_slot_num: state_checkpoint.visible_slot_num,
            rollup_height: state_checkpoint.rollup_height,
            finalized_slot_policy: FinalizedSlotPolicy::UseExplicit(latest_finalized_slot_number),
        }
    }

    /// Apply the given `TxChangeSet` to the `ConcurrentStateCheckpoint`.
    pub fn apply_tx_changes(&self, changeset: TxChangeSet) {
        let mut writer = self.writes.write();
        writer.extend(changeset.writes.into_iter().map(|(k, v)| (k, v.clone())));

        writer.commit();
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
