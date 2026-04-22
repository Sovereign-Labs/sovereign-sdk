use anyhow::{anyhow, bail, Result};
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
    /// Fails if the credential has a legacy/custom account mapping or is
    /// already authorized for the caller's address.
    InsertCredentialId(
        /// The credential id being registered.
        CredentialId,
    ),

    /// Authorizes `credential` to sign transactions that execute as `address`.
    /// The caller must currently be signing as `address` (i.e.
    /// `context.sender() == address`); this is naturally true after PR 2's
    /// resolver for V1 `target_address = Some(address)` and for V0/target=None
    /// where the signer's credential resolves into `address`. Fails if the
    /// tuple is already authorized.
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
    ) -> Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;

        self.exit_if_credential_exists(&new_credential_id, context.sender(), state)?;

        self.authorize_credential(context.sender(), &new_credential_id, state)?;
        Ok(())
    }

    pub(crate) fn add_credential_to_address(
        &mut self,
        address: S::Address,
        credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;

        let key = AccountOwnerKey::new(address, credential);
        anyhow::ensure!(
            !self
                .account_owners
                .get(&key, state)
                .map_err(|err| anyhow!("Error raised while getting account owner: {err:?}"))?
                .unwrap_or(false),
            "CredentialId already authorized for this address"
        );

        self.authorize_credential(&address, &credential, state)?;
        Ok(())
    }

    pub(crate) fn remove_credential_from_address(
        &mut self,
        address: S::Address,
        credential: CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;

        anyhow::ensure!(
            self.is_authorized_for(&address, &credential, state)
                .map_err(|err| anyhow!("Error raised while checking authorization: {err:?}"))?,
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
    ) -> Result<()> {
        self.ensure_custom_account_mappings_enabled(state)?;
        self.ensure_caller_owns(&address, context)?;

        anyhow::ensure!(
            self.is_authorized_for(&address, &old_credential, state)
                .map_err(|err| anyhow!("Error raised while checking authorization: {err:?}"))?,
            "CredentialId is not authorized for this address"
        );

        let new_key = AccountOwnerKey::new(address, new_credential);
        anyhow::ensure!(
            !self
                .account_owners
                .get(&new_key, state)
                .map_err(|err| anyhow!("Error raised while getting account owner: {err:?}"))?
                .unwrap_or(false),
            "CredentialId already authorized for this address"
        );

        self.revoke_credential(&address, &old_credential, state)?;
        self.authorize_credential(&address, &new_credential, state)?;
        Ok(())
    }

    fn revoke_credential(
        &mut self,
        address: &S::Address,
        credential: &CredentialId,
        state: &mut impl TxState<S>,
    ) -> Result<()> {
        let key = AccountOwnerKey::new(*address, *credential);
        self.account_owners.set(&key, &false, state)?;

        if self
            .accounts
            .get(credential, state)
            .map_err(|err| anyhow!("Error raised while getting account: {err:?}"))?
            .is_some_and(|account| account.addr == *address)
        {
            self.accounts.delete(credential, state)?;
        }

        Ok(())
    }

    fn ensure_custom_account_mappings_enabled(
        &self,
        state: &mut impl StateReader<User>,
    ) -> Result<()> {
        if !self
            .enable_custom_account_mappings
            .get(state)
            .map_err(|err| anyhow!("Error reading enable_custom_account_mappings: {err:?}"))?
            .expect(
                "`enable_custom_account_mappings` should not be None; it must be set at genesis.",
            )
        {
            bail!("Custom account mappings are disabled");
        }
        Ok(())
    }

    /// Enforces that the caller is currently signing as `address`, i.e.
    /// `context.sender() == address`. PR 2's authorizer has already proven
    /// the caller controls a credential authorized for `context.sender()`
    /// (either via `is_authorized` on the V1 target path, or via the
    /// credential's natural resolution to `sender` on V0/target=None).
    fn ensure_caller_owns(&self, address: &S::Address, context: &Context<S>) -> Result<()> {
        anyhow::ensure!(
            context.sender() == address,
            "Caller is not authorized to modify credentials for this address"
        );
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
            !self
                .account_owners
                .get(&AccountOwnerKey::new(*address, *new_credential_id), state)
                .map_err(|err| anyhow!("Error raised while getting account owner: {err:?}"))?
                .unwrap_or(false),
            "CredentialId already authorized for this address"
        );
        Ok(())
    }
}
