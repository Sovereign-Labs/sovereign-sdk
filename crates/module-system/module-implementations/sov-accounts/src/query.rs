//! Read-only REST endpoints for inspecting `sov-accounts` state.
//!
//! Intended for migration verification and debugging. The legacy
//! `_accounts` lookup exists so operators can confirm the source map is
//! drained after running [`crate::migrations::apply_legacy_account_migration`].

use std::str::FromStr;

use axum::routing::get;
use sov_modules_api::prelude::{axum, UnwrapInfallible};
use sov_modules_api::rest::utils::{errors, ApiResult, Path};
use sov_modules_api::rest::{ApiState, HasCustomRestApi};
use sov_modules_api::{ApiStateAccessor, CredentialId, Spec};

use crate::{AccountOwnerKey, Accounts};

/// Response of `GET /authorizations/{address}/{credential_id}`.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct AuthorizationResponse {
    /// `true` iff `(address, credential_id)` has an explicit `account_owners`
    /// entry. Does not include the canonical-address fallback.
    pub authorized: bool,
}

/// Response of `GET /legacy/{credential_id}`.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
#[serde(bound = "Addr: serde::Serialize + serde::de::DeserializeOwned")]
pub struct LegacyAccountResponse<Addr> {
    /// The address from the legacy `_accounts` map for this credential, if any.
    /// Expected to be `null` for every credential after the legacy-accounts
    /// migration has run.
    pub legacy_address: Option<Addr>,
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
            .account_owners
            .get(&AccountOwnerKey::new(address, credential_id), &mut accessor)
            .unwrap_infallible()
            .unwrap_or(false);
        Ok(AuthorizationResponse { authorized }.into())
    }

    async fn route_legacy_account(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
        Path(credential_id_str): Path<String>,
    ) -> ApiResult<LegacyAccountResponse<S::Address>> {
        let credential_id = CredentialId::from_str(&credential_id_str).map_err(|e| {
            errors::bad_request_400(
                &format!("invalid credential_id `{credential_id_str}`"),
                e.to_string(),
            )
        })?;

        let legacy_address = state
            ._accounts
            .get(&credential_id, &mut accessor)
            .unwrap_infallible()
            .map(|a| a.addr);
        Ok(LegacyAccountResponse { legacy_address }.into())
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
            .route("/legacy/{credential_id}", get(Self::route_legacy_account))
            .with_state(state.with(self.clone()))
    }
}
