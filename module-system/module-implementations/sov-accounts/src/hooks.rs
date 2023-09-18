use sov_modules_api::hooks::TxHooks;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, Spec, WorkingSet};

use crate::Accounts;

impl<C: Context> TxHooks for Accounts<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<C>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        let pub_key = tx.pub_key().clone();

        let account = match self.accounts.get(&pub_key, working_set) {
            Some(acc) => Ok(acc),
            None => self.create_default_account(pub_key, working_set),
        }?;

        let tx_nonce = tx.nonce();
        let account_nonce = account.nonce;
        anyhow::ensure!(
            account_nonce == tx_nonce,
            "Tx bad nonce, expected: {account_nonce}, but found: {tx_nonce}",
        );
        Ok(account.addr)
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        let mut account = self.accounts.get_or_err(tx.pub_key(), working_set)?;
        account.nonce += 1;
        self.accounts.set(tx.pub_key(), &account, working_set);
        Ok(())
    }
}
