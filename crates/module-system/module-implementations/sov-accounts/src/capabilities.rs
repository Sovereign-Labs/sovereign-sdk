use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, StateWriter};
use sov_state::User;

use crate::{AccountOwnerKey, Accounts};

impl<S: Spec> Accounts<S> {
    /// Resolve the sender's public key to an address.
    ///
    /// Resolution order:
    /// 1. If `(default_address, credential_id)` is present in `account_owners`, return `default_address`.
    /// 2. Else if the legacy `accounts` map has an entry for `credential_id`, return that address
    ///    (pre-upgrade fallback; never write-forwarded).
    /// 3. Else auto-register the tuple in `account_owners` and return `default_address`.
    pub fn resolve_sender_address<ST: StateAccessor>(
        &mut self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, <ST as StateWriter<User>>::Error> {
        let default_key = AccountOwnerKey::new(*default_address, *credential_id);
        if self.account_owners.get(&default_key, state)?.is_some() {
            return Ok(*default_address);
        }
        if let Some(legacy) = self.accounts.get(credential_id, state)? {
            return Ok(legacy.addr);
        }
        self.account_owners.set(&default_key, &true, state)?;
        Ok(*default_address)
    }

    /// Resolve the sender's public key to an address (read-only). Mirrors
    /// [`Self::resolve_sender_address`] steps 1 and 2; step 3 falls through to
    /// returning `default_address` without writing.
    pub fn resolve_sender_address_read_only<ST: StateReader<User>>(
        &self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, ST::Error> {
        if self
            .account_owners
            .get(
                &AccountOwnerKey::new(*default_address, *credential_id),
                state,
            )?
            .is_some()
        {
            return Ok(*default_address);
        }
        if let Some(legacy) = self.accounts.get(credential_id, state)? {
            return Ok(legacy.addr);
        }
        Ok(*default_address)
    }

    /// Returns `true` if `credential_id` is authorized to sign transactions
    /// that execute as `address`. Read-only lookup against
    /// [`Self::account_owners`].
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
