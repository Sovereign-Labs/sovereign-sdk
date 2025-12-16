#![allow(dead_code)] // TODO: Remove this once the implementation is complete.
use std::sync::Arc;

use sov_rollup_interface::common::{RollupHeight, VisibleSlotNumber};
use sov_state::{Namespace, SlotKey, SlotValue, StateGetter};

use crate::{Spec, StateCheckpoint, TxChangeSet};

/// An analogue of `StateCheckpoint` that can be safely written while concurrent reads are happening.
pub struct ConcurrentStateCheckpoint<S: Spec> {
    pub(super) storage: S::Storage,
    pub(crate) uncomitted_changes: Option<Box<dyn StateGetter>>,
    pub(crate) writes: Arc<concread::hashmap::HashMap<(SlotKey, Namespace), Option<SlotValue>>>,
    pub(super) visible_slot_num: VisibleSlotNumber,
    pub(super) rollup_height: RollupHeight,
}

impl<S: Spec> ConcurrentStateCheckpoint<S> {
    /// Create a `ConcurrentStateCheckpoint` containing the same changes as the given `StateCheckpoint`.
    pub fn from_state_checkpoint(mut state_checkpoint: StateCheckpoint<S>) -> Self {
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

        Self {
            storage: state_checkpoint.delta.inner,
            uncomitted_changes: state_checkpoint.delta.uncomitted_changes,
            writes: Arc::new(map),
            visible_slot_num: state_checkpoint.visible_slot_num,
            rollup_height: state_checkpoint.rollup_height,
        }
    }

    /// Apply the given `TxChangeSet` to the `ConcurrentStateCheckpoint`.
    pub fn apply_tx_changes(&self, changeset: TxChangeSet) {
        let mut writer = self.writes.write();
        for ((key, namespace), value) in changeset.writes {
            writer.insert((key.clone(), namespace), value.clone());
        }
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
}
