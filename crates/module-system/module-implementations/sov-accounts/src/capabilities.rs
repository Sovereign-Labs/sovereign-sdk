use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, StateWriter};
use sov_state::User;

use crate::{AccountOwnerKey, Accounts};

impl<S: Spec> Accounts<S> {
    /// Authorizes `credential_id` to sign as `address`.
    pub(crate) fn authorize_credential<ST: StateWriter<User>>(
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

    /// Resolve the sender's public key to an address. If `credential_id` has a
    /// legacy/custom mapping in `accounts`, return it. Otherwise, return the
    /// supplied `default_address` without writing account state.
    pub fn resolve_sender_address<ST: StateAccessor>(
        &mut self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, <ST as StateWriter<User>>::Error> {
        if let Some(account) = self.accounts.get(credential_id, state)? {
            return Ok(account.addr);
        }
        Ok(*default_address)
    }

    /// Read-only variant of [`Self::resolve_sender_address`]: returns
    /// `default_address` when the credential is unknown, without writing.
    pub fn resolve_sender_address_read_only<ST: StateReader<User>>(
        &self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, ST::Error> {
        if let Some(account) = self.accounts.get(credential_id, state)? {
            return Ok(account.addr);
        }
        Ok(*default_address)
    }

    /// Returns `true` if `credential_id` is authorized to sign transactions
    /// that execute as `address`.
    pub fn is_authorized<ST: StateReader<User>>(
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

    /// Returns `true` if `credential_id` is authorized to act as `address`
    /// under any supported relation: legacy/custom `accounts` mapping,
    /// stateless canonical address, or explicit `account_owners`
    /// authorization.
    pub fn is_authorized_for<ST: StateReader<User>>(
        &self,
        address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<bool, ST::Error> {
        if let Some(account) = self.accounts.get(credential_id, state)? {
            return Ok(account.addr == *address);
        }

        let canonical_address: S::Address = (*credential_id).into();
        if canonical_address == *address {
            return Ok(true);
        }

        self.is_authorized(address, credential_id, state)
    }
}
