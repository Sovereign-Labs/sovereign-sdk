use crate::runtime::Runtime;
use sov_modules_api::{
    hooks::{ApplyBlobHooks, TxHooks},
    transaction::Transaction,
    Context, Spec,
};
use sov_modules_stf_template::SequencerOutcome;
use sov_rollup_interface::da::BlobTransactionTrait;
use sov_state::WorkingSet;

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

impl<C: Context> ApplyBlobHooks for Runtime<C> {
    type Context = C;
    type BlobResult = SequencerOutcome;

    fn begin_blob_hook(
        &self,
        blob: &mut impl BlobTransactionTrait,
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
            SequencerOutcome::Rewarded(reward) => {
                self.sequencer_registry.end_blob_hook(reward, working_set)
            }
            SequencerOutcome::Ignored => self.sequencer_registry.end_blob_hook(0, working_set),
            SequencerOutcome::Slashed(_) => Ok(()),
        }
    }
}
