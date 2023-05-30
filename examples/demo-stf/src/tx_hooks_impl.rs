use crate::runtime::Runtime;
use anyhow::Result;
use sov_default_stf::{TxHooks, VerifiedTx};
use sov_modules_api::{hooks::ApplyBatchHooks, Context, Spec};
use sov_state::WorkingSet;

pub struct AppVerifiedTx<C: Context> {
    pub(crate) pub_key: C::PublicKey,
    pub(crate) sender: C::Address,
    pub(crate) runtime_msg: Vec<u8>,
}

impl<C: Context> VerifiedTx for AppVerifiedTx<C> {
    type Address = C::Address;

    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn runtime_message(&self) -> &[u8] {
        &self.runtime_msg
    }
}
impl<C: Context> ApplyBatchHooks for Runtime<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: sov_modules_api::hooks::Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        self.accounts.pre_dispatch_tx_hook(tx, working_set)
    }

    fn post_dispatch_tx_hook(
        &self,
        pub_key: <Self::Context as Spec>::PublicKey,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) {
        self.accounts
            .post_dispatch_tx_hook(pub_key.clone(), working_set);
        self.sequencer.post_dispatch_tx_hook(pub_key, working_set)
    }

    fn enter_apply_blob(
        &self,
        sequencer: &[u8],
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.accounts.enter_apply_blob(sequencer, working_set)?;
        self.sequencer.enter_apply_blob(sequencer, working_set)
    }

    fn exit_apply_blob(
        &self,
        amount: u64,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.accounts.exit_apply_blob(amount, working_set)?;
        self.sequencer.exit_apply_blob(amount, working_set)
    }
}
