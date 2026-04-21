use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, StateWriter};
use sov_state::User;

use crate::{Account, AccountOwnerKey, Accounts};

impl<S: Spec> Accounts<S> {
    /// Writes both indices that together assert `credential_id` is authorized
    /// to sign as `address`. Callers must maintain this pairing so that
    /// [`Self::is_authorized`] can rely on `account_owners` alone.
    pub(crate) fn register_credential<ST: StateAccessor>(
        &mut self,
        address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<(), <ST as StateWriter<User>>::Error> {
        self.accounts
            .set(credential_id, &Account { addr: *address }, state)?;
        self.account_owners.set(
            &AccountOwnerKey::new(*address, *credential_id),
            &true,
            state,
        )
    }

    /// Resolve the sender's public key to an address. If `credential_id` is
    /// unknown, register it to `default_address` and return that.
    pub fn resolve_sender_address<ST: StateAccessor>(
        &mut self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, <ST as StateWriter<User>>::Error> {
        if let Some(account) = self.accounts.get(credential_id, state)? {
            return Ok(account.addr);
        }
        self.register_credential(default_address, credential_id, state)?;
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
}
