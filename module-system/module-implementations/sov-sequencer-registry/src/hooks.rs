use sov_modules_api::hooks::ApplyBlobHooks;
use sov_modules_api::Context;
use sov_rollup_interface::da::BlobTransactionTrait;
use sov_state::WorkingSet;

use crate::{SequencerOutcome, SequencerRegistry};

impl<C: Context> ApplyBlobHooks for SequencerRegistry<C> {
    type Context = C;
    type BlobResult = SequencerOutcome;

    fn begin_blob_hook(
        &self,
        blob: &mut impl BlobTransactionTrait,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        // Clone to satisfy StateMap API
        // TODO: can be fixed after https://github.com/Sovereign-Labs/sovereign-sdk/issues/427
        let sender = blob.sender().as_ref().to_vec();
        self.allowed_sequencers.get_or_err(&sender, working_set)?;
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
                self.allowed_sequencers.delete(&sequencer, working_set);
            }
        }
        Ok(())
    }
}
