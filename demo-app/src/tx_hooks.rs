use crate::tx_verifier::{Transaction, VerifiedTx};
use sov_modules_api::{Context, Spec};
use sov_state::WorkingSet;
use std::marker::PhantomData;

///
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
        let acc = acc_hooks.get_or_create_default_account(tx.pub_key.clone())?;

        let tx_nonce = tx.nonce;
        let acc_nonce = acc.nonce;
        anyhow::ensure!(
            acc_nonce == tx_nonce,
            "Tx bad nonce, expected: {acc_nonce}, but found: {tx_nonce}",
        );

        Ok(VerifiedTx {
            pub_key: tx.pub_key,
            sender: acc.addr,
            runtime_msg: tx.runtime_msg,
        })
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: VerifiedTx<Self::Context>,
        working_set: WorkingSet<<Self::Context as Spec>::Storage>,
    ) {
        let mut acc_hooks = accounts::hooks::Hooks::<Self::Context>::new(working_set);

        acc_hooks
            .inc_nonce(&tx.pub_key)
            // At this point we are sure, that the account corresponding to the tx.pub_key is in the db,
            // therefore this panic should never happen, we add it for sanity check.
            .unwrap_or_else(|e| panic!("Inconsistent nonce {e}"));
    }
}
