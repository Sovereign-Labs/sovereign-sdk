use internals::Delta;
use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};
/// Provides specialized working set wrappers for dealing with protected state.
use sov_state::{SlotKey, SlotValue};

use self::checkpoints::StateCheckpoint;
use super::{checkpoints, internals, Namespace, UniversalStateAccessor};
use crate::capabilities::{Kernel, RollupHeight};
use crate::state::traits::{PrivilegedKernelAccessor, VersionReader};
use crate::{AccessoryStateWriter, GasMeter, Spec};

/// A special wrapper over a `Delta` on the storage that allows access to kernel values to bootstrap the [`StateCheckpoint`].
pub struct BootstrapWorkingSet<'a, S: Spec> {
    /// The inner working set
    pub(super) inner: &'a mut Delta<S::Storage>,
}

impl<S: Spec> UniversalStateAccessor for BootstrapWorkingSet<'_, S> {
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        self.inner.get_size(namespace, key)
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        self.inner.get(namespace, key)
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.inner.set(namespace, key, value);
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        self.inner.delete(namespace, key);
    }
}

impl<S: Spec> GasMeter for BootstrapWorkingSet<'_, S> {
    type Spec = S;
}

/// A special wrapper over [`StateCheckpoint`] that allows access to kernel values
///
/// ## Note
/// This struct implements [`VersionReader`], and the value returned by
/// [`VersionReader::rollup_height_to_access`] is the last known rollup height.
#[derive(Debug)]
pub struct KernelStateAccessor<'a, S: Spec> {
    /// The inner working set
    pub checkpoint: &'a mut StateCheckpoint<S>,
    pub(crate) true_slot_num: SlotNumber,
}

impl<S: Spec> GasMeter for KernelStateAccessor<'_, S> {
    type Spec = S;
}

impl<S: Spec> VersionReader for KernelStateAccessor<'_, S> {
    fn current_visible_slot_number(&self) -> VisibleSlotNumber {
        self.checkpoint.current_visible_slot_number()
    }

    fn max_allowed_slot_number_to_access(&self) -> SlotNumber {
        self.true_slot_num
    }

    fn rollup_height_to_access(&self) -> RollupHeight {
        self.checkpoint.rollup_height
    }
}

impl<S: Spec> PrivilegedKernelAccessor for KernelStateAccessor<'_, S> {
    fn true_slot_number(&self) -> SlotNumber {
        self.true_slot_num
    }
}

impl<S: Spec> AccessoryStateWriter for KernelStateAccessor<'_, S> {}

impl<'a, S: Spec> KernelStateAccessor<'a, S> {
    /// Instantiates a new [`KernelStateAccessor`].
    pub fn from_checkpoint<K: Kernel<S> + ?Sized>(
        kernel: &K,
        checkpoint: &'a mut StateCheckpoint<S>,
    ) -> Self {
        let mut bootstrap = BootstrapWorkingSet {
            inner: &mut checkpoint.delta,
        };

        let true_slot_num = kernel.true_slot_number(&mut bootstrap);

        Self {
            checkpoint,
            true_slot_num,
        }
    }
}

impl<S: Spec> KernelStateAccessor<'_, S> {
    /// Returns the visible rollup height contained in the accessor
    pub fn visible_slot_number(&self) -> VisibleSlotNumber {
        self.checkpoint.visible_slot_num
    }

    /// Updates the true rollup height contained in the accessor
    pub fn update_true_slot_number(&mut self, true_slot_num: SlotNumber) {
        self.true_slot_num = true_slot_num;
    }

    /// Updates the visible rollup height contained in the accessor
    pub fn update_visible_slot_number(&mut self, visible_slot_num: VisibleSlotNumber) {
        self.checkpoint.visible_slot_num = visible_slot_num;
    }

    /// Updates the visible rollup height contained in the accessor
    pub fn update_rollup_height(&mut self, rollup_height: RollupHeight) {
        self.checkpoint.rollup_height = rollup_height;
    }
}

impl<S: Spec> UniversalStateAccessor for KernelStateAccessor<'_, S> {
    fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32> {
        self.checkpoint.get_size(namespace, key)
    }

    fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue> {
        self.checkpoint.get_value(namespace, key)
    }

    fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue) {
        self.checkpoint.set_value(namespace, key, value);
    }

    fn delete_value(&mut self, namespace: Namespace, key: &SlotKey) {
        self.checkpoint.delete_value(namespace, key);
    }
}
