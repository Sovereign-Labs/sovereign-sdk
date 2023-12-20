use sov_modules_api::hooks::TxHooks;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, StateMapAccessor, WorkingSet};

use crate::{Account, Accounts};

/// The computed addresses of a pre-dispatch tx hook.
pub struct AccountsTxHook<C: Context> {
    /// The tx sender address
    pub sender: C::Address,
    /// The sequencer address
    pub sequencer: C::Address,
}

impl<C: Context> Accounts<C> {
    fn get_or_create_default(
        &self,
        pubkey: &C::PublicKey,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<Account<C>> {
        self.accounts
            .get(pubkey, working_set)
            .map(Ok)
            .unwrap_or_else(|| self.create_default_account(pubkey, working_set))
    }
}

impl<C: Context> TxHooks for Accounts<C> {
    type Context = C;
    type PreArg = C::PublicKey;
    type PreResult = AccountsTxHook<C>;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<C>,
        working_set: &mut WorkingSet<C>,
        sequencer: &C::PublicKey,
    ) -> anyhow::Result<AccountsTxHook<C>> {
        let sender = self.get_or_create_default(tx.pub_key(), working_set)?;
        let sequencer = self.get_or_create_default(sequencer, working_set)?;
        let tx_nonce = tx.nonce();

        anyhow::ensure!(
            sender.nonce == tx_nonce,
            "Tx bad nonce, expected: {}, but found: {}",
            tx_nonce,
            sender.nonce
        );

        Ok(AccountsTxHook {
            sender: sender.addr,
            sequencer: sequencer.addr,
        })
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        _ctx: &C,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        let mut account = self.accounts.get_or_err(tx.pub_key(), working_set)?;
        account.nonce += 1;
        self.accounts.set(tx.pub_key(), &account, working_set);
        Ok(())
    }
}
