use crate::runtime::Runtime;
use sov_default_stf::SequencerOutcome;
use sov_modules_api::{
    hooks::{ApplyBlobHooks, ApplyBlobTxHooks},
    transaction::Transaction,
    Context, Spec,
};
use sov_state::WorkingSet;

impl<C: Context> ApplyBlobTxHooks for Runtime<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<Self::Context>,
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
        sequencer: &[u8],
        raw_blob: &[u8],
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.sequencer
            .begin_blob_hook(sequencer, raw_blob, working_set)
    }

    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.sequencer.end_blob_hook(result.reward, working_set)
    }
}
