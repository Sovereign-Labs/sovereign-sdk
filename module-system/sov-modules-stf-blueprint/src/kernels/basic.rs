//! The basic kernel provides censorship resistance by processing all blobs immediately in the order they appear on DA
use sov_blob_storage::BlobStorage;
use sov_chain_state::ChainState;
use sov_modules_api::runtime::capabilities::{
    BlobRefOrOwned, BlobSelector, Kernel, KernelSlotHooks,
};
use sov_modules_api::{Context, DaSpec, WorkingSet};
use sov_state::Storage;

/// The simplest imaginable kernel. It does not do any batching or reordering of blobs.
pub struct BasicKernel<C: Context, Da: DaSpec> {
    phantom: std::marker::PhantomData<C>,
    chain_state: ChainState<C, Da>,
    blob_storage: BlobStorage<C, Da>,
}

impl<C: Context, Da: DaSpec> Default for BasicKernel<C, Da> {
    fn default() -> Self {
        Self {
            phantom: std::marker::PhantomData,
            chain_state: Default::default(),
            blob_storage: Default::default(),
        }
    }
}

impl<C: Context, Da: DaSpec> Kernel<C, Da> for BasicKernel<C, Da> {
    fn true_height(&self, working_set: &mut WorkingSet<C>) -> u64 {
        // let kernel_ws = KernelWorkingSet::from_kernel(self, working_set);
        self.chain_state.true_slot_height(working_set)
    }
    fn visible_height(&self, working_set: &mut WorkingSet<C>) -> u64 {
        self.chain_state.visible_slot_height(working_set)
    }

    type GenesisConfig = ();

    #[cfg(feature = "native")]
    type GenesisPaths = ();

    #[cfg(feature = "native")]
    fn genesis_config(
        genesis_paths: &Self::GenesisPaths,
    ) -> Result<Self::GenesisConfig, anyhow::Error> {
        todo!()
    }

    fn init(&mut self, working_set: &mut sov_modules_api::WorkingSet<C>) {
        todo!()
    }
}

impl<C: Context, Da: DaSpec> BlobSelector<Da> for BasicKernel<C, Da> {
    type Context = C;

    fn get_blobs_for_this_slot<'a, 'k, I>(
        &self,
        current_blobs: I,
        _working_set: &mut sov_modules_api::KernelWorkingSet<'k, Self::Context>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, Da::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        self.blob_storage
            .get_blobs_for_this_slot(current_blobs, _working_set)
    }
}

impl<C: Context, Da: DaSpec> KernelSlotHooks<C, Da> for BasicKernel<C, Da> {
    fn begin_slot_hook(
        &self,
        slot_header: &<Da as DaSpec>::BlockHeader,
        validity_condition: &<Da as DaSpec>::ValidityCondition,
        pre_state_root: &<<Self::Context as sov_modules_api::Spec>::Storage as Storage>::Root,
        working_set: &mut sov_modules_api::WorkingSet<Self::Context>,
    ) {
        let mut ws = sov_modules_api::KernelWorkingSet::from_kernel(self, working_set);
        self.chain_state
            .begin_slot_hook(slot_header, validity_condition, pre_state_root, &mut ws);
    }

    fn end_slot_hook(&self, working_set: &mut sov_modules_api::WorkingSet<Self::Context>) {
        let mut ws = sov_modules_api::KernelWorkingSet::from_kernel(self, working_set);
        self.chain_state.end_slot_hook(&mut ws);
    }
}
