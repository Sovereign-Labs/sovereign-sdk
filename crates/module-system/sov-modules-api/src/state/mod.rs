mod accessors;
mod traits;

#[cfg(test)]
mod tests;

#[cfg(any(feature = "test-utils", feature = "evm"))]
pub use accessors::UnmeteredStateWrapper;
pub use accessors::{
    AccessoryDelta, BootstrapWorkingSet, BorshSerializedSize, ChangeSet, GenesisStateAccessor,
    KernelStateAccessor, PreExecWorkingSet, RevertableTxState, StateCheckpoint, StateProvider,
    TxChangeSet, TxScratchpad, WorkingSet,
};
#[cfg(feature = "native")]
pub use accessors::{AccessoryStateCheckpoint, ApiStateAccessor, ApiStateAccessorError};
#[cfg(feature = "native")]
use sov_rollup_interface::ProvableHeightTracker;
pub use sov_state::TypeErasedEvent;
#[cfg(feature = "native")]
pub use traits::ProvenStateAccessor;
pub use traits::{
    AccessoryStateReader, AccessoryStateReaderAndWriter, AccessoryStateWriter, GenesisState,
    InfallibleKernelStateAccessor, InfallibleStateAccessor, InfallibleStateReaderAndWriter,
    PerBlockCache, PrivilegedKernelAccessor, ProvableStateReader, ProvableStateWriter,
    StateAccessor, StateAccessorError, StateReader, StateReaderAndWriter, StateWriter, TxState,
    VersionReader,
};

#[cfg(feature = "native")]
/// Utilities to allow tracking the maximum provable height of the rollup.
pub mod provable_height_tracker {
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
}
