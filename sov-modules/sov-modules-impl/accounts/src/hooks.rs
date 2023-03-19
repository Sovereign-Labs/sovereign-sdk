use crate::{Account, Accounts, Address};
use anyhow::Result;
use sov_modules_api::ModuleInfo;
use sov_modules_api::PublicKey;
use sov_modules_api::{Context, Spec};
use sov_state::WorkingSet;

pub struct Hooks<C: sov_modules_api::Context> {
    inner: Accounts<C>,
}

impl<C: Context> Hooks<C> {
    pub fn new(storage: WorkingSet<C::Storage>) -> Self {
        Self {
            inner: Accounts::new(storage),
        }
    }

    pub fn get_account_or_create_default(&mut self, pub_key: C::PublicKey) -> Result<Account> {
        match self.inner.accounts.get(&pub_key) {
            Some(acc) => return Ok(acc),
            None => {
                let default_address = pub_key.to_address();
                self.exit_if_address_exists(&default_address)?;
                let new_account = Account {
                    addr: default_address,
                    nonce: 0,
                };

                self.inner.accounts.set(&pub_key, new_account.clone());
                self.inner.public_keys.set(&default_address, pub_key);

                Ok(new_account)
            }
        }
    }

    pub fn inc_nonce(&mut self, pub_key: &C::PublicKey) -> Result<()> {
        let mut account = self.inner.accounts.get_or_err(pub_key)?;
        account.nonce += 1;
        self.inner.accounts.set(pub_key, account);

        Ok(())
    }

    fn exit_if_address_exists(&self, address: &Address) -> Result<()> {
        anyhow::ensure!(
            self.inner.public_keys.get(address).is_none(),
            "Address already exists"
        );
        Ok(())
    }
}
