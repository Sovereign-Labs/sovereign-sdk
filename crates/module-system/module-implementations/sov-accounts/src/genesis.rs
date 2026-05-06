use anyhow::{bail, Result};
use schemars::JsonSchema;
use serde_with::{serde_as, DisplayFromStr};
use sov_modules_api::prelude::*;
use sov_modules_api::{CredentialId, GenesisState};

use crate::{AccountOwnerKey, Accounts};

/// Credential/address authorization data for genesis.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountData<Address> {
    /// Credential ID to authorize.
    #[serde_as(as = "DisplayFromStr")]
    pub credential_id: CredentialId,
    /// Address the credential may act as.
    pub address: Address,
}

/// Initial configuration for sov-accounts module.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(bound = "S: ::sov_modules_api::Spec", rename = "AccountConfig")]
pub struct AccountConfig<S: Spec> {
    /// Credential/address authorizations to initialize.
    pub accounts: Vec<AccountData<S::Address>>,
    /// Enable configured credential authorizations and `InsertCredentialId`.
    #[serde(default = "default_true")]
    pub enable_custom_account_mappings: bool,
}

fn default_true() -> bool {
    true
}

impl<S: Spec> Accounts<S> {
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as sov_modules_api::Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        self.enable_custom_account_mappings
            .set(&config.enable_custom_account_mappings, state)?;

        if !config.enable_custom_account_mappings {
            if !config.accounts.is_empty() {
                bail!("Custom account mapping is disabled, but accounts are provided")
            }
            return Ok(());
        }

        for acc in &config.accounts {
            let key = AccountOwnerKey::new(acc.address, acc.credential_id);
            if self.account_owners.get(&key, state)?.is_some() {
                bail!(
                    "Authorization already exists for address {} and credential {}",
                    acc.address,
                    acc.credential_id
                )
            }
            self.authorize_credential(&acc.address, &acc.credential_id, state)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_modules_api::PublicKey;
    use sov_test_utils::{TestPublicKey, TestSpec};

    use super::*;

    #[test]
    fn test_config_serialization() {
        let pub_key = &TestPublicKey::from_str(
            "1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf",
        )
        .unwrap();

        let credential_id = pub_key.credential_id();

        let config = AccountConfig::<TestSpec> {
            accounts: vec![AccountData {
                credential_id,
                address: credential_id.into(),
            }],
            enable_custom_account_mappings: false,
        };

        let data = r#"
        {
            "accounts":[{"credential_id":"0x1cd4e2d9d5943e6f3d12589d31feee6bb6c11e7b8cd996a393623e207da72cbf","address":"sov1rn2w9kw4jslx70gjtzwnrlhwdwmvz8nm3nvedgunvglzqp593hk"}],
            "enable_custom_account_mappings":false
        }"#;

        let parsed_config: AccountConfig<TestSpec> = serde_json::from_str(data).unwrap();
        assert_eq!(parsed_config, config);
    }
}
