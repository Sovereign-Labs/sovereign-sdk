use crate::tx_verifier::{Transaction, VerifiedTx};
use sov_modules_api::{Context, Spec};
use sov_state::WorkingSet;
use std::marker::PhantomData;

pub(crate) trait TxHooks {
    type Context: Context;

    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<Self::Context>,
        working_set: WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<VerifiedTx<Self::Context>>;

    fn post_dispatch_tx_hook(
        &self,
        tx: VerifiedTx<Self::Context>,
        working_set: WorkingSet<<Self::Context as Spec>::Storage>,
    );
}

pub(crate) struct DemoAppTxHooks<C: Context> {
    _p: PhantomData<C>,
}

impl<C: Context> DemoAppTxHooks<C> {
    pub fn new() -> Self {
        Self {
            _p: Default::default(),
        }
    }
}

impl<C: Context> TxHooks for DemoAppTxHooks<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<Self::Context>,
        working_set: WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<VerifiedTx<Self::Context>> {
        let mut acc_hooks = accounts::hooks::Hooks::<Self::Context>::new(working_set);
        let acc = acc_hooks.get_account_or_create_default(tx.pub_key.clone())?;

        anyhow::ensure!(tx.nonce == acc.nonce, "");

        Ok(VerifiedTx {
            sender: tx.pub_key,
            runtime_msg: tx.runtime_msg,
            nonce: tx.nonce,
        })
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: VerifiedTx<Self::Context>,
        working_set: WorkingSet<<Self::Context as Spec>::Storage>,
    ) {
        let mut acc_hooks = accounts::hooks::Hooks::<Self::Context>::new(working_set);
        //acc_hooks.inc_nonce(t);
    }
}
