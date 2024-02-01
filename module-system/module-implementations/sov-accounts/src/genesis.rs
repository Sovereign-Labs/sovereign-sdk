use anyhow::{bail, Result};
use sov_modules_api::{Context, PublicKey, StateMapAccessor, WorkingSet};

use crate::{Account, Accounts};

/// Initial configuration for sov-accounts module.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound = "C::PublicKey: serde::Serialize + serde::de::DeserializeOwned")]
pub struct AccountConfig<C: Context> {
    /// Public keys to initialize the rollup.
    pub pub_keys: Vec<C::PublicKey>,
}

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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_modules_api::default_context::DefaultContext;
    use sov_modules_api::default_signature::DefaultPublicKey;

    use super::*;

    #[test]
    fn test_config_serialization() {
        let pub_key = &DefaultPublicKey::from_str(
            "1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf",
        )
        .unwrap();

        let config = AccountConfig::<DefaultContext> {
            pub_keys: vec![pub_key.clone()],
        };

        let data = r#"
        {
            "pub_keys":["1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf"]
        }"#;

        let parsed_config: AccountConfig<DefaultContext> = serde_json::from_str(data).unwrap();
        assert_eq!(parsed_config, config);
    }
}
