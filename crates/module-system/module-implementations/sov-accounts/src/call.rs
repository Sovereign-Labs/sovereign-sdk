use anyhow::{bail, Context as _};
use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CredentialId, Spec, StateReader, TxState};
use sov_state::namespaces::User;

use crate::{AccountOwnerKey, Accounts};

/// Represents the available call messages for interacting with the sov-accounts module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
pub enum CallMessage<S: Spec> {
    /// Authorizes `credential_id` as a signer for the caller's address.
    /// Fails if the credential is already authorized for the caller's address.
    InsertCredentialId(
        /// The credential id being authorized.
        CredentialId,
    ),

    /// Authorizes `credential` to sign transactions that execute as `address`.
    /// The caller must currently be signing as `address`. Fails if the tuple
    /// is already authorized.
    AddCredentialToAddress {
        /// The address whose credential set is being extended. Must equal
        /// `context.sender()`.
        address: S::Address,
        /// The credential being authorized for `address`.
        credential: CredentialId,
    },

    /// Revokes `credential` from `address`. The caller must currently be
    /// signing as `address`. No orphan guard: revoking the last credential
    /// succeeds and leaves `address` unspendable via this map.
    RemoveCredentialFromAddress {
        /// The address whose credential set is being reduced. Must equal
        /// `context.sender()`.
        address: S::Address,
        /// The credential being revoked from `address`.
        credential: CredentialId,
    },

    /// Atomically swaps `old_credential` for `new_credential` on `address`.
    /// Functionally equivalent to a `RemoveCredentialFromAddress` followed by
    /// an `AddCredentialToAddress`, collapsed into a single call so the
    /// caller does not have to authorize two transactions during a rotation.
    /// The caller must currently be signing as `address`.
    RotateCredentialOnAddress {
        /// The address whose credential set is being rotated. Must equal
        /// `context.sender()`.
        address: S::Address,
        /// The credential being revoked from `address`. Must be currently
        /// authorized.
        old_credential: CredentialId,
        /// The credential being authorized for `address`. Must not already
        /// be authorized.
        new_credential: CredentialId,
    },
}

impl<S: Spec> Accounts<S> {
    pub(crate) fn insert_credential_id(
        &mut self,
        new_credential_id: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;

        self.ensure_credential_not_authorized(context.sender(), &new_credential_id, state)?;

        self.authorize_credential(context.sender(), &new_credential_id, state)?;
        Ok(())
    }

    pub(crate) fn add_credential_to_address(
        &mut self,
        address: S::Address,
        credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;
        self.ensure_credential_not_authorized(&address, &credential, state)?;

        self.authorize_credential(&address, &credential, state)?;
        Ok(())
    }

    pub(crate) fn remove_credential_from_address(
        &mut self,
        address: S::Address,
        credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;

        anyhow::ensure!(
            self.is_authorized_for(&address, &credential, state)
                .context("Failed to check credential authorization")?,
            "CredentialId is not authorized for this address"
        );

        self.revoke_credential(&address, &credential, state)?;
        Ok(())
    }

    pub(crate) fn rotate_credential_on_address(
        &mut self,
        address: S::Address,
        old_credential: CredentialId,
        new_credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;

        anyhow::ensure!(
            self.is_authorized_for(&address, &old_credential, state)
                .context("Failed to check old credential authorization")?,
            "old_credential is not authorized for this address"
        );
        self.ensure_credential_not_authorized(&address, &new_credential, state)?;

        self.revoke_credential(&address, &old_credential, state)?;
        self.authorize_credential(&address, &new_credential, state)?;
        Ok(())
    }

    fn revoke_credential(
        &mut self,
        address: &S::Address,
        credential: &CredentialId,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let key = AccountOwnerKey::new(*address, *credential);
        // Write `false` explicitly so the canonical-address fallback in
        // `is_authorized_for` cannot re-authorize the tuple.
        self.account_owners.set(&key, &false, state)?;
        Ok(())
    }

    fn ensure_custom_account_mappings_enabled(
        &self,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        if !self
            .enable_custom_account_mappings
            .get(state)
            .context("Failed to read enable_custom_account_mappings")?
            .expect(
                "`enable_custom_account_mappings` should not be None; it must be set at genesis.",
            )
        {
            bail!("Custom account mappings are disabled");
        }
        Ok(())
    }

    /// Enforces that the caller is signing as `address`. The upstream
    /// authorization path has already verified the caller controls a
    /// credential authorized for `context.sender()`.
    fn ensure_caller_owns(&self, address: &S::Address, context: &Context<S>) -> anyhow::Result<()> {
        let sender = context.sender();
        anyhow::ensure!(
            sender == address,
            "Caller {sender} is not authorized to modify credentials for address {address}"
        );
        Ok(())
    }

    fn ensure_credential_not_authorized(
        &self,
        address: &S::Address,
        credential: &CredentialId,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self
                .is_authorized_for(address, credential, state)
                .context("Failed to check credential authorization")?,
            "CredentialId already authorized for this address"
        );
        Ok(())
    }
}
