#![allow(dead_code)] // TODO: Remove this once the implementation is complete.
use std::sync::Arc;

use sov_rollup_interface::common::{RollupHeight, SlotNumber, VisibleSlotNumber};
use sov_state::{Namespace, NativeStorage, SlotKey, SlotValue, StateGetter};

use crate::{Spec, StateCheckpoint, TxChangeSet};

/// An analogue of `StateCheckpoint` that can be safely written while concurrent reads are happening.
pub struct ConcurrentStateCheckpoint<S: Spec> {
    pub(super) storage: S::Storage,
    pub(crate) uncomitted_changes: Option<Box<dyn StateGetter>>,
    pub(crate) writes: Arc<concread::hashmap::HashMap<(SlotKey, Namespace), Option<SlotValue>>>,
    pub(super) visible_slot_num: VisibleSlotNumber,
    pub(super) rollup_height: RollupHeight,
    pub(super) latest_finalized_slot_number: SlotNumber,
}

impl<S: Spec> ConcurrentStateCheckpoint<S> {
    /// Create a `ConcurrentStateCheckpoint` containing the same changes as the given `StateCheckpoint`.
    pub fn from_state_checkpoint(state_checkpoint: StateCheckpoint<S>) -> Self {
        let latest_finalized_slot_number = state_checkpoint.delta.inner.latest_version();
        Self::from_state_checkpoint_with_finalized_slot(
            state_checkpoint,
            latest_finalized_slot_number,
        )
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

        Self {
            storage: state_checkpoint.delta.inner,
            uncomitted_changes: state_checkpoint.delta.uncomitted_changes,
            writes: Arc::new(map),
            visible_slot_num: state_checkpoint.visible_slot_num,
            rollup_height: state_checkpoint.rollup_height,
            latest_finalized_slot_number: latest_finalized_slot_number.min(max_available_slot),
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
        self.latest_finalized_slot_number
    }
}
