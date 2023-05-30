use crate::runtime::Runtime;
use sov_modules_api::{
    hooks::{ApplyBlobSequencerHooks, ApplyBlobTxHooks},
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

impl<C: Context> ApplyBlobSequencerHooks for Runtime<C> {
    type Context = C;

    fn lock_sequencer_bond(
        &self,
        sequencer: &[u8],
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.sequencer.lock_sequencer_bond(sequencer, working_set)
    }

    fn reward_sequencer(
        &self,
        amount: u64,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.sequencer.reward_sequencer(amount, working_set)
    }
}
