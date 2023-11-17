//! The basic kernel provides censorship resistance by processing all blobs immediately in the order they appear on DA
use sov_modules_api::runtime::capabilities::{BlobRefOrOwned, BlobSelector, Kernel};
use sov_modules_api::{Context, DaSpec};

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

impl<C: Context, Da: DaSpec> Kernel<C, Da> for BasicKernel<C> {
    fn true_height(&self) -> u64 {
        todo!()
    }
    fn visible_height(&self) -> u64 {
        todo!()
    }
}

impl<C: Context, Da: DaSpec> BlobSelector<Da> for BasicKernel<C> {
    type Context = C;

    fn get_blobs_for_this_slot<'a, I>(
        &self,
        current_blobs: I,
        _working_set: &mut sov_modules_api::WorkingSet<Self::Context>,
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
