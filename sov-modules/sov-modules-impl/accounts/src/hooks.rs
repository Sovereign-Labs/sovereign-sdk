use crate::{Account, Accounts, Address};
use anyhow::Result;
use sov_modules_api::Context;
use sov_modules_api::ModuleInfo;
use sov_modules_api::PublicKey;
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
        &mut self,
        pub_key: C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<Account> {
        match self.inner.accounts.get(&pub_key, working_set) {
            Some(acc) => Ok(acc),
            None => {
                let default_address = pub_key.to_address();
                self.exit_if_address_exists(&default_address, working_set)?;

                let new_account = Account {
                    addr: default_address,
                    nonce: 0,
                };

                self.inner.accounts.set(&pub_key, new_account, working_set);
                self.inner
                    .public_keys
                    .set(&default_address, pub_key, working_set);
                Ok(new_account)
            }
        }
    }

    pub fn inc_nonce(
        &mut self,
        pub_key: &C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let mut account = self.inner.accounts.get_or_err(pub_key, working_set)?;
        account.nonce += 1;
        self.inner.accounts.set(pub_key, account, working_set);

        Ok(())
    }

    fn exit_if_address_exists(
        &self,
        address: &Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.inner.public_keys.get(address, working_set).is_none(),
            "Address already exists"
        );
        Ok(())
    }
}
