use crate::{Account, Accounts};
use anyhow::Result;
use sov_modules_api::Context;
use sov_modules_api::ModuleInfo;
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
