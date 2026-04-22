use anyhow::{anyhow, bail, Result};
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
    /// Authorizes `credential_id` as a signer for the caller's address.
    /// Fails if the credential has a legacy/custom account mapping or is
    /// already authorized for the caller's address.
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
    ) -> Result<()> {
        if !self.enable_custom_account_mappings.get(state)?.expect(
            "`enable_custom_account_mappings` should not be None; it must be set at genesis.",
        ) {
            bail!("Custom account mappings are disabled");
        }

        self.exit_if_credential_exists(&new_credential_id, context.sender(), state)?;

        self.authorize_credential(context.sender(), &new_credential_id, state)?;
        Ok(())
    }

    fn exit_if_credential_exists(
        &self,
        new_credential_id: &CredentialId,
        address: &S::Address,
        state: &mut impl StateReader<User>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.accounts
                .get(new_credential_id, state)
                .map_err(|err| anyhow!("Error raised while getting account: {err:?}"))?
                .is_none(),
            "New CredentialId already exists"
        );
        anyhow::ensure!(
            self.account_owners
                .get(&AccountOwnerKey::new(*address, *new_credential_id), state)
                .map_err(|err| anyhow!("Error raised while getting account owner: {err:?}"))?
                .is_none(),
            "CredentialId already authorized for this address"
        );
        Ok(())
    }
}
