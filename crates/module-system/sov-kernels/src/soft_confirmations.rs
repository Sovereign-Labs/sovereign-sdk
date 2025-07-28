//! The basic kernel provides censorship resistance by processing all blobs immediately in the order they appear on DA

use std::convert::Infallible;

use sov_blob_storage::BlobStorage;
use sov_chain_state::ChainState;
use sov_modules_api::capabilities::{BlobSelectorOutput, BlockGasInfo, RollupHeight};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::runtime::capabilities::{BlobSelector, Kernel as KernelTrait};
#[cfg(feature = "native")]
use sov_modules_api::AccessoryStateReaderAndWriter;
use sov_modules_api::{
    BootstrapWorkingSet, DaSpec, Gas, HexHash, InjectedControlFlow, IterableBatchWithId,
    KernelStateAccessor, SelectedBlob, Spec, StateReader, VersionReader, VisibleSlotNumber,
};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::RelevantBlobIters;
use sov_state::{Kernel, Storage, User};

/// A kernel supporting based sequencing with soft confirmations
pub struct SoftConfirmationsKernel<'a, S: Spec> {
    pub chain_state: &'a mut ChainState<S>,
    pub blob_storage: &'a mut BlobStorage<S>,
}

impl<S: Spec> KernelTrait<S> for SoftConfirmationsKernel<'_, S> {
    fn true_slot_number(&self, state: &mut BootstrapWorkingSet<'_, S>) -> SlotNumber {
        self.chain_state.true_slot_number_at_bootstrap(state)
    }
    fn next_visible_slot_number(
        &self,
        state: &mut BootstrapWorkingSet<'_, S>,
    ) -> VisibleSlotNumber {
        self.chain_state.next_visible_slot_number(state)
    }

    fn rollup_height(&self, state: &mut BootstrapWorkingSet<'_, S>) -> RollupHeight {
        self.chain_state.rollup_height(state).unwrap_infallible()
    }

    fn record_gas_usage(
        &mut self,
        state: &mut sov_modules_api::StateCheckpoint<S>,
        final_gas_info: BlockGasInfo<S::Gas>,
        rollup_height: RollupHeight,
    ) {
        self.chain_state
            .record_gas_usage(state, final_gas_info, rollup_height);
    }
}

impl<S: Spec> BlobSelector for SoftConfirmationsKernel<'_, S> {
    type Spec = S;

    const ACCEPTS_PREFERRED_BATCHES: bool = true;

    fn get_blobs_for_this_slot<CF: InjectedControlFlow<Self::Spec> + Clone>(
        &mut self,
        current_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        state: &mut KernelStateAccessor<'_, S>,
        cf: CF,
    ) -> anyhow::Result<(
        BlobSelectorOutput<SelectedBlob<S, IterableBatchWithId<S, CF>>>,
        Vec<HexHash>,
    )> {
        self.blob_storage
            .get_blobs_for_this_slot(current_blobs, state, cf)
    }

    fn get_non_preferred_blobs<CF: InjectedControlFlow<Self::Spec> + Clone>(
        &mut self,
        slot_range: impl Iterator<Item = SlotNumber>,
        state: &mut KernelStateAccessor<'_, Self::Spec>,
        cf: CF,
    ) -> Vec<SelectedBlob<Self::Spec, IterableBatchWithId<S, CF>>> {
        self.blob_storage
            .get_non_preferred_blobs(slot_range, state)
            .into_iter()
            .map(|blob| blob.into_selected_blob(cf.clone()))
            .collect()
    }

    #[cfg(feature = "native")]
    fn escrow_funds_for_preferred_sequencer(
        &mut self,
        amount: sov_modules_api::Amount,
        state: &mut KernelStateAccessor<'_, S>,
    ) -> anyhow::Result<()> {
        self.blob_storage
            .escrow_funds_for_preferred_sequencer(amount, state)
    }

    fn next_sequence_number(&self, state: &mut KernelStateAccessor<'_, Self::Spec>) -> u64 {
        self.blob_storage.next_sequence_number(state)
    }
}

impl<S: Spec> sov_modules_api::capabilities::ChainState for SoftConfirmationsKernel<'_, S> {
    type Spec = S;

    fn synchronize_chain(
        &mut self,
        slot_header: &<S::Da as DaSpec>::BlockHeader,
        pre_state_root: &<S::Storage as Storage>::Root,
        state: &mut sov_modules_api::KernelStateAccessor<S>,
    ) {
        self.chain_state
            .synchronize_chain(slot_header, pre_state_root, state);
    }

    fn finalize_chain_state(
        &mut self,
        gas_used: &S::Gas,
        state: &mut sov_modules_api::KernelStateAccessor<S>,
    ) {
        self.chain_state.finalize_chain_state(gas_used, state);
    }

    fn base_fee_per_gas<
        Reader: VersionReader
            + StateReader<Kernel, Error = Infallible>
            + StateReader<User, Error = Infallible>,
    >(
        &self,
        state: &mut Reader,
    ) -> Option<<<Self::Spec as Spec>::Gas as Gas>::Price> {
        self.chain_state.base_fee_per_gas(state).unwrap_infallible()
    }

    fn block_gas_limit<
        Reader: VersionReader
            + StateReader<Kernel, Error = Infallible>
            + StateReader<User, Error = Infallible>,
    >(
        &self,
        state: &mut Reader,
    ) -> Option<<Self::Spec as Spec>::Gas> {
        self.chain_state.block_gas_limit(state).unwrap_infallible()
    }

    fn visible_hash_for(
        &self,
        rollup_height: RollupHeight,
        state: &mut sov_modules_api::KernelStateAccessor<S>,
    ) -> Option<<<Self::Spec as Spec>::Storage as Storage>::Root> {
        self.chain_state.visible_hash_for(rollup_height, state)
    }

    fn increment_rollup_height(
        &mut self,
        state: &mut KernelStateAccessor<'_, Self::Spec>,
        visible_slot_number: VisibleSlotNumber,
    ) {
        self.chain_state
            .increment_rollup_height(state, visible_slot_number);
    }

    #[cfg(feature = "native")]
    fn visible_hash_with_accessory_state(
        &self,
        rollup_height: RollupHeight,
        state: &mut sov_modules_api::AccessoryDelta<<Self::Spec as Spec>::Storage>,
    ) -> Option<<<Self::Spec as Spec>::Storage as Storage>::Root> {
        self.chain_state
            .visible_hash_with_accessory_state(rollup_height, state)
    }

    #[cfg(feature = "native")]
    fn save_user_state_root(
        &mut self,
        height: RollupHeight,
        root: [u8; 32],
        state: &mut KernelStateAccessor<'_, Self::Spec>,
    ) {
        self.chain_state.save_user_state_root(height, root, state);
    }

    #[cfg(feature = "native")]
    fn save_genesis_root(
        &mut self,
        state: &mut impl AccessoryStateReaderAndWriter,
        genesis_root: &<<Self::Spec as Spec>::Storage as Storage>::Root,
    ) {
        self.chain_state.save_genesis_root(state, genesis_root);
    }

    #[cfg(feature = "native")]
    fn genesis_root(
        &self,
        state: &mut impl AccessoryStateReaderAndWriter,
    ) -> Option<<S::Storage as Storage>::Root> {
        self.chain_state.genesis_root(state)
    }

    fn operating_mode<
        Reader: VersionReader
            + StateReader<User, Error = Infallible>
            + StateReader<Kernel, Error = Infallible>,
    >(
        &self,
        state: &mut Reader,
    ) -> sov_modules_api::OperatingMode {
        self.chain_state.operating_mode(state).unwrap_infallible()
    }
}

/// These methods are used in the tests to access the internal state of the kernel.
/// Normally these should not be used, because everything happens inside the stf.
#[cfg(feature = "test-utils")]
impl<S: Spec> SoftConfirmationsKernel<'_, S> {
    /// Gets a reference to the kernel's ChainState module.
    pub fn get_chain_state(&self) -> &sov_chain_state::ChainState<S> {
        self.chain_state
    }

    /// Gets a reference to the kernel's BlobStorage module.
    pub fn get_blob_storage(&self) -> &sov_blob_storage::BlobStorage<S> {
        self.blob_storage
    }
}
