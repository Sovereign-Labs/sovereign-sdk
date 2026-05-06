//! Read-only REST endpoints for inspecting `sov-accounts` state.

use std::str::FromStr;

use axum::routing::get;
use sov_modules_api::prelude::{axum, UnwrapInfallible};
use sov_modules_api::rest::utils::{errors, ApiResult, Path};
use sov_modules_api::rest::{ApiState, HasCustomRestApi};
use sov_modules_api::{ApiStateAccessor, CredentialId, Spec};

use crate::Accounts;

/// Response of `GET /authorizations/{address}/{credential_id}`.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct AuthorizationResponse {
    /// `true` iff `credential_id` is authorized to act as `address`,
    /// including the canonical-address fallback.
    pub authorized: bool,
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
        Ok(AuthorizationResponse { authorized }.into())
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
            StateCheckpoint::<S>::new(storage, kernel.as_ref(), None),
        ));
        let (_sender, receiver) = sov_modules_api::prelude::tokio::sync::watch::channel(checkpoint);

        let accounts = Accounts::<S>::default();
        let state = ApiState::build(Arc::new(()), receiver, kernel, None).with(accounts);
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

        assert!(response.0.authorized);
    }
}
