use crate::{Account, Accounts};
use anyhow::Result;
use sov_modules_api::hooks::ApplyBatchHooks;
use sov_modules_api::hooks::Transaction;
use sov_modules_api::Context;
use sov_modules_api::ModuleInfo;
use sov_modules_api::Spec;
use sov_state::WorkingSet;

pub struct Hooks<C: sov_modules_api::Context> {
    inner: Accounts<C>,
}

impl<C: Context> Hooks<C> {
    pub fn new() -> Self {
        Self {
            inner: Accounts::new(),
        }
    }

    pub fn get_or_create_default_account(
        &self,
        pub_key: C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<Account<C>> {
        match self.inner.accounts.get(&pub_key, working_set) {
            Some(acc) => Ok(acc),
            None => self.inner.create_default_account(pub_key, working_set),
        }
    }

    pub fn inc_nonce(
        &self,
        pub_key: &C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let mut account = self.inner.accounts.get_or_err(pub_key, working_set)?;
        account.nonce += 1;
        self.inner.accounts.set(pub_key, account, working_set);

        Ok(())
    }
}

impl<C: Context> ApplyBatchHooks for Accounts<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<C>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        let pub_key = tx.pub_key;

        let acc = match self.accounts.get(&pub_key, working_set) {
            Some(acc) => Ok(acc),
            None => self.create_default_account(pub_key, working_set),
        }?;

        let tx_nonce = tx.nonce;
        let acc_nonce = acc.nonce;
        anyhow::ensure!(
            acc_nonce == tx_nonce,
            "Tx bad nonce, expected: {acc_nonce}, but found: {tx_nonce}",
        );

        Ok(acc.addr)
    }

    fn post_dispatch_tx_hook(
        &self,
        pub_key: <Self::Context as Spec>::PublicKey,
        working_set: &mut WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) {
        //TODO
        let mut account = self.accounts.get_or_err(&pub_key, working_set).unwrap();
        account.nonce += 1;
        self.accounts.set(&pub_key, account, working_set);
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
