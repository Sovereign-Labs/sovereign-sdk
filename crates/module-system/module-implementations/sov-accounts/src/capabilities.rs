use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, StateWriter};
use sov_state::User;

use crate::{Account, AccountOwnerKey, Accounts};

impl<S: Spec> Accounts<S> {
    /// Resolve the sender's public key to an address.
    ///
    /// Resolution order:
    /// 1. If `accounts` has an entry for `credential_id`, return that address.
    /// 2. Else auto-register `credential_id` to `default_address` in both
    ///    `accounts` and `account_owners`, then return `default_address`.
    pub fn resolve_sender_address<ST: StateAccessor>(
        &mut self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, <ST as StateWriter<User>>::Error> {
        if let Some(account) = self.accounts.get(credential_id, state)? {
            return Ok(account.addr);
        }

        let default_key = AccountOwnerKey::new(*default_address, *credential_id);
        self.accounts.set(
            credential_id,
            &Account {
                addr: *default_address,
            },
            state,
        )?;
        self.account_owners.set(&default_key, &true, state)?;
        Ok(*default_address)
    }

    /// Resolve the sender's public key to an address (read-only). Mirrors the
    /// lookup in [`Self::resolve_sender_address`]; auto-registration falls
    /// through to returning `default_address` without writing.
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
        if self
            .account_owners
            .get(&AccountOwnerKey::new(*address, *credential_id), state)?
            .is_some()
        {
            return Ok(true);
        }

        Ok(matches!(
            self.accounts.get(credential_id, state)?,
            Some(Account { addr }) if addr == *address
        ))
    }
}
