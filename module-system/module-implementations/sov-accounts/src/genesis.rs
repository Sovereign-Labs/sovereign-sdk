use anyhow::{bail, Result};
use sov_modules_api::{PublicKey, WorkingSet};

use crate::{Account, Accounts};

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        for pub_key in config.pub_keys.iter() {
            if self.accounts.get(pub_key, working_set).is_some() {
                bail!("Account already exists")
            }

            self.create_default_account(pub_key, working_set)?;
        }

        Ok(())
    }

    pub(crate) fn create_default_account(
        &self,
        pub_key: &C::PublicKey,
        working_set: &mut WorkingSet<C>,
    ) -> Result<Account<C>> {
        let default_address = pub_key.to_address();
        self.exit_if_address_exists(&default_address, working_set)?;

        let new_account = Account {
            addr: default_address.clone(),
            nonce: 0,
        };

        self.accounts.set(pub_key, &new_account, working_set);

        self.public_keys.set(&default_address, pub_key, working_set);
        Ok(new_account)
    }

    fn exit_if_address_exists(
        &self,
        address: &C::Address,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.public_keys.get(address, working_set).is_none(),
            "Address already exists"
        );
        Ok(())
    }
}
