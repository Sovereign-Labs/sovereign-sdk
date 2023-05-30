use crate::Accounts;
use sov_modules_api::hooks::ApplyBlobTxHooks;
use sov_modules_api::hooks::Transaction;
use sov_modules_api::Context;

use sov_modules_api::Spec;
use sov_state::WorkingSet;

impl<C: Context> ApplyBlobTxHooks for Accounts<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<C>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        let pub_key = tx.pub_key().clone();

        let acc = match self.accounts.get(&pub_key, working_set) {
            Some(acc) => Ok(acc),
            None => self.create_default_account(pub_key, working_set),
        }?;

        let tx_nonce = tx.nonce();
        let acc_nonce = acc.nonce;
        anyhow::ensure!(
            acc_nonce == tx_nonce,
            "Tx bad nonce, expected: {acc_nonce}, but found: {tx_nonce}",
        );

        Ok(acc.addr)
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        let mut account = self.accounts.get_or_err(tx.pub_key(), working_set)?;
        account.nonce += 1;
        self.accounts.set(tx.pub_key(), account, working_set);
        Ok(())
    }

    fn enter_apply_blob(
        &self,
        sequencer: &[u8],
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn exit_apply_blob(
        &self,
        amount: u64,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
