use sov_modules_api::{CredentialId, Spec, StateReader, StateWriter};
use sov_state::User;

use crate::{AccountOwnerKey, Accounts};

impl<S: Spec> Accounts<S> {
    /// Authorizes `credential_id` to sign as `address`.
    pub fn authorize_credential<ST: StateWriter<User>>(
        &mut self,
        address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<(), <ST as StateWriter<User>>::Error> {
        self.account_owners.set(
            &AccountOwnerKey::new(*address, *credential_id),
            &true,
            state,
        )
    }

    /// Resolve the sender's credential to an address.
    ///
    /// Returns `default_address` unconditionally. The legacy `accounts` map is
    /// no longer consulted; operators must run the legacy-accounts migration
    /// (see [`crate::migrations`]) before deploying a binary that includes
    /// this code on a chain with pre-upgrade entries.
    pub fn resolve_sender_address<ST: StateReader<User>>(
        &mut self,
        default_address: &S::Address,
        _credential_id: &CredentialId,
        _state: &mut ST,
    ) -> Result<S::Address, ST::Error> {
        Ok(*default_address)
    }

    /// Read-only variant of [`Self::resolve_sender_address`].
    pub fn resolve_sender_address_read_only<ST: StateReader<User>>(
        &self,
        default_address: &S::Address,
        _credential_id: &CredentialId,
        _state: &mut ST,
    ) -> Result<S::Address, ST::Error> {
        Ok(*default_address)
    }

    /// Returns `true` only if `(address, credential_id)` has an explicit entry
    /// in `account_owners`. For the full authorization check including the
    /// canonical fallback, use [`Self::is_authorized_for`].
    pub fn is_explicitly_authorized<ST: StateReader<User>>(
        &self,
        address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<bool, ST::Error> {
        Ok(self
            .account_owners
            .get(&AccountOwnerKey::new(*address, *credential_id), state)?
            .is_some())
    }

    /// Returns `true` if `credential_id` is authorized to act as `address`.
    ///
    /// Returns `true` when `address` is the canonical address of
    /// `credential_id` (i.e. `credential_id.into() == address`) or when an
    /// explicit `account_owners` authorization exists.
    pub fn is_authorized_for<ST: StateReader<User>>(
        &self,
        address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<bool, ST::Error> {
        let canonical_address: S::Address = (*credential_id).into();
        if canonical_address == *address {
            return Ok(true);
        }

        self.is_explicitly_authorized(address, credential_id, state)
    }
}
