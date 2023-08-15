use sov_modules_api::hooks::ApplyBlobHooks;
use sov_modules_api::Context;
use sov_rollup_interface::da::BlobReaderTrait;
use sov_state::WorkingSet;

use crate::{SequencerOutcome, SequencerRegistry};

impl<C: Context, B: BlobReaderTrait> ApplyBlobHooks<B> for SequencerRegistry<C> {
    type Context = C;
    type BlobResult = SequencerOutcome;

    fn begin_blob_hook(
        &self,
        blob: &mut B,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        if !self.is_sender_allowed(&blob.sender(), working_set) {
            anyhow::bail!("sender {} is not allowed to submit blobs", blob.sender());
        }
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
