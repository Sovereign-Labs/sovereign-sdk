use anyhow::anyhow;
use anyhow::bail;
use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CredentialId, Spec, StateReader, TxState};
use sov_state::namespaces::User;

use crate::{AccountOwnerKey, Accounts};

/// Represents the available call messages for interacting with the sov-accounts module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
pub enum CallMessage {
    /// Authorizes a credential to sign transactions that execute as the
    /// caller's address. Does not affect the credential's stateless default
    /// routing (`credential_id.into()`).
    InsertCredentialId(
        /// The credential id being authorized.
        CredentialId,
    ),
}

impl<S: Spec> Accounts<S> {
    pub(crate) fn insert_credential_id(
        &mut self,
        new_credential_id: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if !self.enable_custom_account_mappings.get(state)?.expect(
            "`enable_custom_account_mappings` should not be None; it must be set at genesis.",
        ) {
            bail!("Custom account mappings are disabled");
        }

        let key = AccountOwnerKey::new(*context.sender(), new_credential_id);
        self.exit_if_authorization_exists(&key, state)?;
        self.account_owners.set(&key, &true, state)?;
        Ok(())
    }

    fn exit_if_authorization_exists(
        &self,
        key: &AccountOwnerKey<S>,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.account_owners
                .get(key, state)
                .map_err(|err| anyhow!("Error raised while getting account owner: {err:?}"))?
                .is_none(),
            "CredentialId already authorized for this address"
        );
        Ok(())
    }
}
