//! The basic kernel provides censorship resistance by processing all blobs immediately in the order they appear on DA
use sov_modules_api::{
    capabilities::{BlobSelector, Kernel},
    Context, DaSpec,
};

/// The simplest imaginable kernel. It does not do any batching or reordering of blobs.
pub struct BasicKernel<C> {
    phantom: std::marker::PhantomData<C>,
}

impl<C: Context> Default for BasicKernel<C> {
    fn default() -> Self {
        Self {
            phantom: std::marker::PhantomData,
        }
    }
}

impl<C: Context, Da: DaSpec> Kernel<C, Da> for BasicKernel<C> {}

impl<C: Context, Da: DaSpec> BlobSelector<Da> for BasicKernel<C> {
    type Context = C;

    fn get_blobs_for_this_slot<'a, I>(
        &self,
        current_blobs: I,
        _working_set: &mut sov_modules_api::WorkingSet<Self::Context>,
    ) -> anyhow::Result<Vec<sov_modules_api::capabilities::BlobRefOrOwned<'a, Da::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        Ok(current_blobs
            .into_iter()
            .map(|b| sov_modules_api::capabilities::BlobRefOrOwned::Ref(b))
            .collect())
    }
}
