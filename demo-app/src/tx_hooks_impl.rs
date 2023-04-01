use sov_app_template::{TxHooks, VerifiedTx};
use sov_modules_api::{Context, Spec};
use sov_state::WorkingSet;

use crate::tx_verifier_impl::Transaction;

pub(crate) struct AppVerifiedTx<C: Context> {
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

pub(crate) struct DemoAppTxHooks<C: Context> {
    accounts_hooks: accounts::hooks::Hooks<C>,
}

impl<C: Context> DemoAppTxHooks<C> {
    pub fn new() -> Self {
        Self {
            accounts_hooks: accounts::hooks::Hooks::<C>::new(),
        }
    }
}

impl<C: Context> TxHooks for DemoAppTxHooks<C> {
    type Context = C;
    type Transaction = Transaction<C>;
    type VerifiedTx = AppVerifiedTx<C>;

    fn pre_dispatch_tx_hook(
        &mut self,
        tx: Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<Self::VerifiedTx> {
        let addr = self.check_nonce_for_address(tx.nonce, tx.pub_key.clone(), working_set)?;

        Ok(AppVerifiedTx {
            pub_key: tx.pub_key,
            sender: addr,
            runtime_msg: tx.runtime_msg,
        })
    }

    fn post_dispatch_tx_hook(
        &mut self,
        tx: Self::VerifiedTx,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) {
        self.accounts_hooks
            .inc_nonce(&tx.pub_key, working_set)
            // At this point we are sure, that the account corresponding to the tx.pub_key is in the db,
            // therefore this panic should never happen, we add it for sanity check.
            .unwrap_or_else(|e| panic!("Inconsistent nonce {e}"));
    }
}

impl<C: Context> DemoAppTxHooks<C> {
    fn check_nonce_for_address(
        &mut self,
        tx_nonce: u64,
        tx_pub_key: C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<C::Address> {
        let acc = self
            .accounts_hooks
            .get_or_create_default_account(tx_pub_key, working_set)?;

        let acc_nonce = acc.nonce;
        anyhow::ensure!(
            acc_nonce == tx_nonce,
            "Tx bad nonce, expected: {acc_nonce}, but found: {tx_nonce}",
        );

        Ok(acc.addr)
    }
}
