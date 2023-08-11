use sov_modules_api::hooks::ApplyBlobHooks;
use sov_modules_api::Context;
use sov_rollup_interface::da::BlobReaderTrait;
use sov_state::WorkingSet;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use zk_cycle_macros::cycle_tracker;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use zk_cycle_utils::print_cycle_count;

use crate::{SequencerOutcome, SequencerRegistry};

impl<C: Context> ApplyBlobHooks for SequencerRegistry<C> {
    type Context = C;
    type BlobResult = SequencerOutcome;

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn begin_blob_hook(
        &self,
        blob: &mut impl BlobReaderTrait,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        // Clone to satisfy StateMap API
        // TODO: can be fixed after https://github.com/Sovereign-Labs/sovereign-sdk/issues/427
        let sender = blob.sender().as_ref().to_vec();
        #[cfg(all(target_os = "zkvm", feature = "bench"))]
        print_cycle_count();
        self.allowed_sequencers.get_or_err(&sender, working_set)?;
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
