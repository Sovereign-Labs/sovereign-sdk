//! Read-only REST endpoints for inspecting `sov-accounts` state.

use std::str::FromStr;

use axum::routing::get;
use sov_modules_api::prelude::{axum, UnwrapInfallible};
use sov_modules_api::rest::utils::{errors, ApiResult, Path};
use sov_modules_api::rest::{ApiState, HasCustomRestApi};
use sov_modules_api::{ApiStateAccessor, CredentialId, Spec};

use crate::Accounts;

/// Response of `GET /authorizations/{address}/{credential_id}`.
///
/// Exposes the three distinct authorization predicates from
/// [`crate::Accounts`]. Prefer `admit_as_override` and `admit_as_default`
/// over the legacy `authorized` field — the new fields mirror the on-chain
/// admit-path directly, while `authorized` collapses two distinct semantics
/// into a single boolean and can disagree with what the chain actually does
/// for authenticators (such as EVM) whose `default_address` is not the
/// credential's canonical address.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct AuthorizationResponse {
    /// `true` iff `credential_id` is authorized to act as `address` under
    /// the canonical-fallback semantic of [`Accounts::is_authorized_for`].
    ///
    /// Equivalent to `admit_as_override || (no_entry && canonical(credential_id) == address)`.
    ///
    /// Retained for backward compatibility. Prefer `admit_as_override` /
    /// `admit_as_default` for new consumers; they correspond directly to
    /// what the on-chain admit-path checks.
    #[deprecated(
        note = "prefer admit_as_override (admit-path override semantic) and admit_as_default (admit-path default semantic); see field docs"
    )]
    pub authorized: bool,

    /// `true` iff the on-chain admit-path would accept a transaction
    /// submitted with `address_override = Some(this_address)` and this
    /// credential. Matches [`Accounts::is_explicitly_authorized`]: there
    /// must be an explicit `true` entry in `account_owners`.
    pub admit_as_override: bool,

    /// `true` iff the on-chain admit-path would accept a transaction
    /// submitted with `address_override = None` whose authenticator declares
    /// this address as `default_address` for this credential. Matches
    /// [`Accounts::is_default_address_authorized`].
    ///
    /// NOTE: returns `true` for any `(address, credential)` pair with no
    /// `account_owners` entry — the chain trusts the authenticator to have
    /// verified the credential→default_address binding before resolution.
    /// Callers should interpret this field as "would the chain admit, IF the
    /// authenticator declares this address as default", not as a standalone
    /// authorization signal.
    pub admit_as_default: bool,
}

impl<S: Spec> Accounts<S> {
    async fn route_is_authorized(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
        Path((address_str, credential_id_str)): Path<(String, String)>,
    ) -> ApiResult<AuthorizationResponse> {
        let address = <S::Address as FromStr>::from_str(&address_str).map_err(|_| {
            errors::bad_request_400(
                &format!("invalid address `{address_str}`"),
                "address parse failed",
            )
        })?;
        let credential_id = CredentialId::from_str(&credential_id_str).map_err(|e| {
            errors::bad_request_400(
                &format!("invalid credential_id `{credential_id_str}`"),
                e.to_string(),
            )
        })?;

        let authorized = state
            .is_authorized_for(&address, &credential_id, &mut accessor)
            .unwrap_infallible();
        let admit_as_override = state
            .is_explicitly_authorized(&address, &credential_id, &mut accessor)
            .unwrap_infallible();
        let admit_as_default = state
            .is_default_address_authorized(&address, &credential_id, &mut accessor)
            .unwrap_infallible();
        #[allow(deprecated)]
        let response = AuthorizationResponse {
            authorized,
            admit_as_override,
            admit_as_default,
        };
        Ok(response.into())
    }
}

impl<S: Spec> HasCustomRestApi for Accounts<S> {
    type Spec = S;

    fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
        axum::Router::new()
            .route(
                "/authorizations/{address}/{credential_id}",
                get(Self::route_is_authorized),
            )
            .with_state(state.with(self.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sov_rollup_interface::node::PrimaryShutdownController;
    use sov_modules_api::capabilities::mocks::MockKernel;
    use sov_modules_api::rest::utils::Path;
    use sov_modules_api::{ConcurrentStateCheckpoint, StateCheckpoint};
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::TestSpec;

    use super::*;

    type S = TestSpec;

    #[test]
    fn route_includes_canonical_fallback() {
        let kernel = Arc::new(MockKernel::<S>::default());
        let storage = SimpleStorageManager::new().create_storage();
        let checkpoint = Arc::new(ConcurrentStateCheckpoint::from_state_checkpoint(
            StateCheckpoint::<S>::new(storage, kernel.as_ref()),
        ));
        let (_sender, receiver) = sov_modules_api::prelude::tokio::sync::watch::channel(checkpoint);

        let accounts = Accounts::<S>::default();
        let state = ApiState::build(Arc::new(()), receiver, kernel, None, Default::default())
            .with(accounts);
        let accessor = state.default_api_state_accessor();

        let credential_id = CredentialId::from([7u8; 32]);
        let address = <S as Spec>::Address::from(credential_id);

        let response = sov_modules_api::prelude::tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(Accounts::<S>::route_is_authorized(
                state,
                accessor,
                Path((address.to_string(), credential_id.to_string())),
            ))
            .unwrap();

        #[allow(deprecated)]
        let authorized = response.0.authorized;
        assert!(
            authorized,
            "canonical (address == credential_id.into()) pair should be authorized under is_authorized_for semantic"
        );
        assert!(
            !response.0.admit_as_override,
            "no explicit account_owners entry exists — admit_as_override must be false"
        );
        assert!(
            response.0.admit_as_default,
            "no explicit account_owners entry exists — admit_as_default must be true (chain trusts authenticator)"
        );
    }
}
