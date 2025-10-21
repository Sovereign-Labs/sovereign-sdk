use anyhow::Result;
use anyhow::{bail, Context as _};
use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CredentialId, Spec, TxState};

use crate::Accounts;

/// Represents the available call messages for interacting with the sov-accounts module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
pub enum CallMessage {
    /// Inserts a new credential id for the corresponding Account.
    InsertCredentialId(
        /// The new credential id.
        CredentialId,
    ),
}

impl<S: Spec> Accounts<S> {
    pub(crate) fn insert_credential_id(
        &mut self,
        new_credential_id: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        if !self.enable_custom_account_mappings.get(state)?.expect(
            "`enable_custom_account_mappings` should not be None; it must be set at genesis.",
        ) {
            bail!("Custom account mappings are disabled");
        }

        // Insert the new credential id -> account mapping
        let mut account = self
            .accounts
            .get(&context.sender(), state)?
            .unwrap_or_default();
        if account.allowed_credentials.contains(&new_credential_id) {
            bail!("Credential already exists in account");
        }
        account
            .allowed_credentials
            .try_push(new_credential_id)
            .with_context(|| {
                format!(
                    "Maximum number of allowed credentials for address {} exceeded",
                    context.sender()
                )
            })?;
        self.accounts.set(context.sender(), &account, state)?;

        Ok(())
    }
}
