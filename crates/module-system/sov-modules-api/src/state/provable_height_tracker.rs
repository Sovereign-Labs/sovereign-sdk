use std::marker::PhantomData;

use sov_rollup_interface::common::SlotNumber;

use super::*;
use crate::capabilities::HasKernel;
use crate::rest::StateUpdateReceiver;
use crate::Spec;
/// A default implementation of [`ProvableHeightTracker`].
/// Tracks the maximum height provable in the rollup by using the kernel of the rollup.
pub struct MaximumProvableHeight<S: Spec, K: HasKernel<S>> {
    state_update_receiver: StateUpdateReceiver<S::Storage>,
    _kernel: PhantomData<K>,
}

impl<S: Spec, K: HasKernel<S>> MaximumProvableHeight<S, K> {
    /// Creates a new [`MaximumProvableHeight`].
    pub fn new(state_update_receiver: StateUpdateReceiver<S::Storage>, _kernel: K) -> Self {
        Self {
            state_update_receiver,
            _kernel: PhantomData,
        }
    }
}

impl<S: Spec, K: HasKernel<S> + Default> ProvableHeightTracker for MaximumProvableHeight<S, K> {
    fn max_provable_slot_number(&self) -> SlotNumber {
        let storage = self.state_update_receiver.borrow().storage.clone();
        let mut kernel = K::default();
        let checkpoint = StateCheckpoint::new(storage, &kernel.kernel());
        // Substract 1 because the state root at slot height `i` is only available at slot height `i + 1`.
        checkpoint
            .current_visible_slot_number()
            .as_true()
            .saturating_sub(1)
    }
}

/// An implementation of [`ProvableHeightTracker`] that can be used to specify an infinite height.
#[cfg(feature = "test-utils")]
#[derive(Clone, Debug, Default)]
pub struct InfiniteHeight;

#[cfg(feature = "test-utils")]
impl InfiniteHeight {}

#[cfg(feature = "test-utils")]
impl ProvableHeightTracker for InfiniteHeight {
    fn max_provable_slot_number(&self) -> SlotNumber {
        SlotNumber::MAX
    }
}
