//! The basic kernel provides censorship resistance by processing all blobs immediately in the order they appear on DA
use sov_blob_storage::BlobStorage;
use sov_chain_state::ChainState;
use sov_modules_api::runtime::capabilities::{
    BlobRefOrOwned, BlobSelector, Kernel, KernelSlotHooks,
};
use sov_modules_api::{Context, DaSpec};
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
    fn true_height(&self) -> u64 {
        todo!()
    }
    fn visible_height(&self) -> u64 {
        todo!()
    }

    type GenesisConfig = ();

    type GenesisPaths = ();

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
        Ok(current_blobs
            .into_iter()
            .map(sov_modules_api::runtime::capabilities::BlobRefOrOwned::Ref)
            .collect())
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
        todo!()
    }

    fn end_slot_hook(&self, working_set: &mut sov_modules_api::WorkingSet<Self::Context>) {
        todo!()
    }
}
