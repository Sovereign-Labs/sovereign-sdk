use crate::tx_verifier::{Transaction, VerifiedTx};
use sov_modules_api::{Address, Context};
use sov_state::WorkingSet;

/// TxHooks allows injecting custom logic into a transaction processing pipeline.
pub(crate) trait TxHooks {
    type Context: Context;

    /// pre_dispatch_tx_hook runs just before a transaction is dispatched to an appropriate module.
    fn pre_dispatch_tx_hook(
        &mut self,
        tx: Transaction<Self::Context>,
    ) -> anyhow::Result<VerifiedTx<Self::Context>>;

    /// post_dispatch_tx_hook runs after the tx is dispatched to an appropriate module.
    fn post_dispatch_tx_hook(&mut self, tx: VerifiedTx<Self::Context>);
}

pub(crate) struct DemoAppTxHooks<C: Context> {
    accounts_hooks: accounts::hooks::Hooks<C>,
}

impl<C: Context> DemoAppTxHooks<C> {
    pub fn new(working_set: WorkingSet<C::Storage>) -> Self {
        Self {
            accounts_hooks: accounts::hooks::Hooks::<C>::new(working_set),
        }
    }
}

impl<C: Context> TxHooks for DemoAppTxHooks<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &mut self,
        tx: Transaction<Self::Context>,
    ) -> anyhow::Result<VerifiedTx<Self::Context>> {
        let addr = self.accounts_hook(tx.nonce, tx.pub_key.clone())?;

        Ok(VerifiedTx {
            pub_key: tx.pub_key,
            sender: addr,
            runtime_msg: tx.runtime_msg,
        })
    }

    fn post_dispatch_tx_hook(&mut self, tx: VerifiedTx<Self::Context>) {
        self.accounts_hooks
            .inc_nonce(&tx.pub_key)
            // At this point we are sure, that the account corresponding to the tx.pub_key is in the db,
            // therefore this panic should never happen, we add it for sanity check.
            .unwrap_or_else(|e| panic!("Inconsistent nonce {e}"));
    }
}

impl<C: Context> DemoAppTxHooks<C> {
    fn accounts_hook(
        &mut self,
        tx_nonce: u64,
        tx_pub_key: C::PublicKey,
    ) -> anyhow::Result<Address> {
        let acc = self
            .accounts_hooks
            .get_or_create_default_account(tx_pub_key)?;

        let acc_nonce = acc.nonce;
        anyhow::ensure!(
            acc_nonce == tx_nonce,
            "Tx bad nonce, expected: {acc_nonce}, but found: {tx_nonce}",
        );

        Ok(acc.addr)
    }
}
