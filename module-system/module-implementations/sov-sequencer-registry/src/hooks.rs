use sov_modules_api::hooks::ApplyBlobHooks;
use sov_modules_api::{BlobReaderTrait, Context};
use sov_state::WorkingSet;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_macros::cycle_tracker;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_utils::print_cycle_count;

use crate::{SequencerOutcome, SequencerRegistry};

impl<C: Context, B: BlobReaderTrait> ApplyBlobHooks<B> for SequencerRegistry<C> {
    type Context = C;
    type BlobResult = SequencerOutcome;

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn begin_blob_hook(
        &self,
        blob: &mut B,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
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
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        match result {
            SequencerOutcome::Completed => (),
            SequencerOutcome::Slashed { sequencer } => {
                self.delete(sequencer, working_set);
            }
        }
        Ok(())
    }
}
