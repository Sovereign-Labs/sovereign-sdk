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

    /// Returns `true` only if `(address, credential_id)` has an explicit
    /// `true` entry in `account_owners`.
    ///
    /// This is the strict semantic used by the on-chain admit-path when a
    /// transaction sets `address_override = Some(_)`. The REST endpoint
    /// `/authorizations/{address}/{credential_id}` exposes this value as the
    /// `admit_as_override` field of `AuthorizationResponse` (in `query`,
    /// native feature only).
    ///
    /// See also:
    /// - [`Self::is_default_address_authorized`] — the permissive semantic
    ///   used when `address_override = None`.
    /// - [`Self::is_authorized_for`] — the canonical-fallback semantic used
    ///   for internal credential lifecycle checks (revoke, rotate, conflict).
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

    /// Returns `true` if the authenticator-selected default address may be used
    /// with `credential_id`.
    ///
    /// Lookup order:
    /// 1. If `account_owners[(address, credential_id)]` has an explicit
    ///    entry, that value is authoritative — including `false`, which
    ///    is how `revoke_credential` denies the default path.
    /// 2. Otherwise, returns `true`: the authenticator's declared default
    ///    address is trusted for the `address_override = None` path.
    ///
    /// This is the permissive semantic used by the on-chain admit-path when a
    /// transaction has `address_override = None` — the chain trusts that the
    /// authenticator has already verified the credential→default_address
    /// binding before resolution. The REST endpoint
    /// `/authorizations/{address}/{credential_id}` exposes this value as the
    /// `admit_as_default` field of `AuthorizationResponse` (in `query`,
    /// native feature only).
    ///
    /// Note: `default_address` may differ from `canonical(credential_id)`.
    /// The EVM authenticator, for example, sets `default_address` to the
    /// `MultiAddress::Vm` variant while `canonical(credential_id)` produces
    /// the `Standard` variant. Callers MUST NOT assume equality.
    ///
    /// See also:
    /// - [`Self::is_explicitly_authorized`] — the strict semantic used when
    ///   `address_override = Some(_)`.
    /// - [`Self::is_authorized_for`] — the canonical-fallback semantic used
    ///   for internal credential lifecycle checks (revoke, rotate, conflict).
    pub fn is_default_address_authorized<ST: StateReader<User>>(
        &self,
        address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<bool, ST::Error> {
        Ok(self
            .account_owners
            .get(&AccountOwnerKey::new(*address, *credential_id), state)?
            .unwrap_or(true))
    }

    /// Returns `true` if `credential_id` is authorized to act as `address`
    /// under the **canonical-fallback** semantic.
    ///
    /// Lookup order:
    /// 1. If `account_owners[(address, credential_id)]` has an explicit
    ///    entry, that value is authoritative — including `false`, which
    ///    is how `revoke_credential` denies the canonical fallback.
    /// 2. Otherwise, returns `true` iff `address` is the canonical address
    ///    of `credential_id` (i.e. `credential_id.into() == address`).
    ///
    /// This semantic is used for **internal credential lifecycle checks** in
    /// the `call` module (revocation, rotation, conflict detection). It is
    /// *not* used by the on-chain admit-path. The REST endpoint
    /// `/authorizations/{address}/{credential_id}` exposes this value as the
    /// (deprecated) `authorized` field of `AuthorizationResponse` (in `query`,
    /// native feature only); new REST consumers should prefer
    /// `admit_as_override` and `admit_as_default` instead, which mirror the
    /// admit-path directly.
    ///
    /// See also:
    /// - [`Self::is_explicitly_authorized`] — matches admit-path override.
    /// - [`Self::is_default_address_authorized`] — matches admit-path default.
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
