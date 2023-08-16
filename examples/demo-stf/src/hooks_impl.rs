use sov_modules_api::hooks::{ApplyBlobHooks, TxHooks};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, Spec};
use sov_modules_stf_template::SequencerOutcome;
use sov_rollup_interface::da::BlobReaderTrait;
use sov_sequencer_registry::SequencerRegistry;
use sov_state::WorkingSet;
use tracing::info;

use crate::runtime::Runtime;

impl<C: Context> TxHooks for Runtime<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        self.accounts.pre_dispatch_tx_hook(tx, working_set)
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.accounts.post_dispatch_tx_hook(tx, working_set)
    }
}

impl<C: Context, B: BlobReaderTrait> ApplyBlobHooks<B> for Runtime<C> {
    type Context = C;
    type BlobResult = SequencerOutcome<B::Address>;

    fn begin_blob_hook(
        &self,
        blob: &mut B,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.sequencer_registry.begin_blob_hook(blob, working_set)
    }

    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        match result {
            SequencerOutcome::Rewarded(_reward) => {
                // TODO: Process reward here or above.
                <SequencerRegistry<C> as ApplyBlobHooks<B>>::end_blob_hook(
                    &self.sequencer_registry,
                    sov_sequencer_registry::SequencerOutcome::Completed,
                    working_set,
                )
            }
            SequencerOutcome::Ignored => Ok(()),
            SequencerOutcome::Slashed {
                reason,
                sequencer_da_address,
            } => {
                info!("Sequencer {} slashed: {:?}", sequencer_da_address, reason);
                <SequencerRegistry<C> as ApplyBlobHooks<B>>::end_blob_hook(
                    &self.sequencer_registry,
                    sov_sequencer_registry::SequencerOutcome::Slashed {
                        sequencer: sequencer_da_address.as_ref().to_vec(),
                    },
                    working_set,
                )
            }
        }
    }
}
