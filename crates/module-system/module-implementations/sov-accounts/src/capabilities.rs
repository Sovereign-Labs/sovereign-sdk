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
            .unwrap_or(false))
    }

    /// Returns `true` if `credential_id` is authorized to act as `address`.
    ///
    /// Lookup order:
    /// 1. If `account_owners[(address, credential_id)]` has an explicit
    ///    entry, that value is authoritative — including `false`, which
    ///    is how `revoke_credential` denies the canonical fallback.
    /// 2. Otherwise, returns `true` iff `address` is the canonical address
    ///    of `credential_id` (i.e. `credential_id.into() == address`).
    pub fn is_authorized_for<ST: StateReader<User>>(
        &self,
        address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<bool, ST::Error> {
        if let Some(is_authorized) = self
            .account_owners
            .get(&AccountOwnerKey::new(*address, *credential_id), state)?
        {
            return Ok(is_authorized);
        }

        let canonical_address: S::Address = (*credential_id).into();
        Ok(canonical_address == *address)
    }
}
