use anyhow::anyhow;
use anyhow::bail;
use schemars::JsonSchema;
use sov_modules_api::digest::Digest;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CredentialId, CryptoSpec, Spec, StateReader, TxState};
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
    ) -> anyhow::Result<()> {
        if !self.enable_custom_account_mappings.get(state)?.expect(
            "`enable_custom_account_mappings` should not be None; it must be set at genesis.",
        ) {
            bail!("Custom account mappings are disabled");
        }

        self.exit_if_credential_exists(&new_credential_id, state)?;

        let mut hasher = <S::CryptoSpec as CryptoSpec>::Hasher::new();
        hasher.update(new_credential_id.0 .0);
        // Mix the sender in so the new credential's address isn't just the sender's own.
        // Defense-in-depth: (a) a multisig registered by A shouldn't alias A's address —
        // otherwise a later compromise of A's single key also controls the multisig; and
        // (b) if credential removal is ever added, this prevents a removed credential
        // from being re-registered by a different sender and colliding with the old address.
        hasher.update(context.sender().as_ref());
        // TODO: also mix in a per-sender nonce so the same (credential, sender) pair
        // can't reproduce an address a compromised sender previously controlled.
        let address_source: [u8; 32] = hasher.finalize().into();

        // Route through CredentialId so each Spec's existing `From<CredentialId>`
        // handles address-width reduction (28 / 32 / 20 bytes).
        let new_non_controlled_address = S::Address::from(CredentialId::from_bytes(address_source));

        // Insert the new credential id -> account mapping
        let account = Account {
            addr: new_non_controlled_address,
        };
        self.accounts.set(&new_credential_id, &account, state)?;

        Ok(())
    }

    fn exit_if_credential_exists(
        &self,
        new_credential_id: &CredentialId,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.accounts
                .get(new_credential_id, state)
                .map_err(|err| anyhow!("Error raised while getting account: {err:?}"))?
                .is_none(),
            "New CredentialId already exists: {new_credential_id}"
        );
        Ok(())
    }
}
