use sov_modules_api::hooks::ApplyBlobHooks;
use sov_modules_api::{BlobReaderTrait, Context, WorkingSet};
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_macros::cycle_tracker;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_utils::print_cycle_count;

use crate::{SequencerOutcome, SequencerRegistry};

impl<C: Context, Da: sov_modules_api::DaSpec> ApplyBlobHooks<Da::BlobTransaction>
    for SequencerRegistry<C, Da>
{
    type Context = C;
    type BlobResult = SequencerOutcome<Da>;

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn begin_blob_hook(
        &self,
        blob: &mut Da::BlobTransaction,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        #[cfg(all(target_os = "zkvm", feature = "bench"))]
        print_cycle_count();
        if !self.is_sender_allowed(&blob.sender(), working_set) {
            anyhow::bail!("sender {} is not allowed to submit blobs", blob.sender());
        }
        #[cfg(all(target_os = "zkvm", feature = "bench"))]
        print_cycle_count();
        Ok(())
    }

    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        match result {
            SequencerOutcome::Completed => (),
            SequencerOutcome::Slashed { sequencer } => {
                self.delete(&sequencer, working_set);
            }
        }
        Ok(())
    }
}
