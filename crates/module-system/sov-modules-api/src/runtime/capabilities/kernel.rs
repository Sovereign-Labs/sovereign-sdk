use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};

use super::{BlockGasInfo, RollupHeight};
use crate::{BootstrapWorkingSet, KernelStateAccessor, Spec, StateCheckpoint};

/// Allows the kernel to map between a rollup height and the visible height at that slot.
/// This is used to enable access to the correct (visible) kernel state during archival queries.
#[cfg(feature = "native")]
pub trait KernelWithSlotMapping<S: Spec>: Sync + Send + 'static {
    /// Gets the visible rollup height as of the given true rollup height.
    // This method takes `ApiStateAccessor` rather than an `impl Trait` because
    // we need it to be object safe
    fn visible_slot_number_at(
        &self,
        true_slot_number: SlotNumber,
        state: &mut crate::state::ApiStateAccessor<S>,
    ) -> Option<VisibleSlotNumber>;

    /// Returns the visible slot number at the given rollup height.
    fn rollup_height_to_visible_slot_number(
        &self,
        height: RollupHeight,
        state: &mut crate::state::ApiStateAccessor<S>,
    ) -> Option<VisibleSlotNumber>;

    /// Returns the associated rollup height given a true slot number.
    fn true_slot_number_to_rollup_height(
        &self,
        slot_number: SlotNumber,
        state: &mut crate::state::ApiStateAccessor<S>,
    ) -> Option<RollupHeight>;

    /// Returns the latest known rollup height.
    fn current_rollup_height(&self, state: &mut crate::state::ApiStateAccessor<S>) -> RollupHeight;

    /// Retrieves the true slot number during which a given rollup height was processed.
    ///
    /// Note that the true slot number for a rollup height is not known until that slot has been
    /// finalized on the DA layer. However, the results of a slot may be known well before that point
    /// (assuming that the sequencer is not malicious). In other words, querying for the true slot number
    /// may return `None` even if the rollup height has already finished processing.
    fn true_slot_number_at_historical_height(
        &self,
        height: RollupHeight,
        state: &mut crate::state::ApiStateAccessor<S>,
    ) -> Option<SlotNumber>;

    /// Returns the base fee per gas accessible at the specified rollup height for this state accessor.
    ///
    /// ## Note
    /// This method may return `None` if it is not possible to retrieve the correct base fee per gas from the state.
    fn base_fee_per_gas_at(
        &self,
        height: super::RollupHeight,
        state: &mut crate::state::ApiStateAccessor<S>,
    ) -> Option<<<S as Spec>::Gas as crate::Gas>::Price>;
}

/// The kernel is responsible for managing the inputs to the `apply_blob` method.
/// A simple implementation will simply process all blobs in the order that they appear,
/// while a second will support a "preferred sequencer" with some limited power to reorder blobs
/// in order to give out soft confirmations.
pub trait Kernel<S: Spec> {
    /// Returns a [`KernelStateAccessor`] for the given [`StateCheckpoint`].
    fn accessor<'a>(&self, state: &'a mut StateCheckpoint<S>) -> KernelStateAccessor<'a, S> {
        KernelStateAccessor::from_checkpoint(self, state)
    }

    /// Return the current rollup height
    fn true_slot_number(&self, state: &mut BootstrapWorkingSet<'_, S>) -> SlotNumber;
    /// Return the next value of the slot number at which transactions currently *appear* to be executing.
    fn next_visible_slot_number(&self, state: &mut BootstrapWorkingSet<'_, S>)
        -> VisibleSlotNumber;

    /// Return the current rollup height
    fn rollup_height(&self, state: &mut BootstrapWorkingSet<'_, S>) -> RollupHeight;

    /// Record the gas usage for a given rollup height.
    fn record_gas_usage(
        &mut self,
        state: &mut StateCheckpoint<S>,
        final_gas_info: BlockGasInfo<S::Gas>,
        rollup_height: RollupHeight,
    );
}
