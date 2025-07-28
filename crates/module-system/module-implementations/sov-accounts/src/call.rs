use anyhow::{anyhow, Result};
use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CredentialId, Spec, StateReader, TxState};
use sov_state::namespaces::User;

use crate::{Account, Accounts};

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
        self.exit_if_credential_exists(&new_credential_id, state)?;

        // Insert the new credential id -> account mapping
        let account = Account {
            addr: context.sender().clone(),
        };
        self.accounts.set(&new_credential_id, &account, state)?;

        Ok(())
    }

    fn exit_if_credential_exists(
        &self,
        new_credential_id: &CredentialId,
        state: &mut impl StateReader<User>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.accounts
                .get(new_credential_id, state)
                .map_err(|err| anyhow!("Error raised while getting account: {err:?}"))?
                .is_none(),
            "New CredentialId already exists"
        );
        Ok(())
    }
}
