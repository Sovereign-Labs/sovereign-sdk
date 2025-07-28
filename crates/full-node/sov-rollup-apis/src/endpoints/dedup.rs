use std::str::FromStr;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use sov_modules_api::prelude::anyhow;
use sov_modules_api::rest::ApiState;
use sov_modules_api::{metered_credential, ApiStateAccessor, CryptoSpec, Spec};
use sov_rest_utils::{errors, preconfigured_router_layers};
use sov_uniqueness::Uniqueness;

/// Trait for the `/rollup/addresses/{address}/dedup` endpoint.
///
/// Rollup developers should implement this to provide dedup functionality to external services
/// such as web3 sdks in a generic way.
pub trait DeDupEndpoint<S: Spec>: Clone + Send + Sync + 'static {
    /// The response data returned by the `dedup` API endpoint.
    type Response: Serialize;

    /// An error that can be returned by the `dedup` endpoint.
    /// Will be included in the response body and can be used to debug failures in client-side
    /// applications.
    type Error: std::fmt::Display;

    /// Handle the `dedup` request.
    fn handler(address: String, state: ApiStateAccessor<S>) -> Result<Self::Response, Self::Error>;

    /// Provides rollup state to the handler.
    fn state(&self) -> ApiStateAccessor<S>;

    /// Returns a configured axum router for the dedup endpoint.
    ///
    /// Calls the implemented [`DeDupEndpoint::handler`] and returns the result.
    /// If [`DeDupEndpoint::handler`] returns a error then it will be included in the
    /// [`sov_rest_utils::ErrorObject`]s details field.
    ///
    /// # Warning
    ///
    /// If you override this method you should ensure you provide the standard dedup path. If the
    /// path is different then external tooling like web3 SDKs won't be able to consume the
    /// functionality and will fail to work.
    fn axum_router(&self) -> Router<()> {
        preconfigured_router_layers(
            Router::new()
                .route(
                    "/rollup/addresses/:address/dedup",
                    get(
                        |Path(address): Path<String>, State(state): State<Self>| async move {
                            match Self::handler(address, state.state()) {
                                Ok(data) => axum::Json(data).into_response(),
                                Err(err) => errors::bad_request_400("Failed to dedup address", err),
                            }
                        },
                    ),
                )
                .with_state(self.clone()),
        )
    }
}

/// Provides the `/rollup/addresses/{address}/dedup` endpoint utilising the sovereign provided
/// `uniqueness` module.
#[derive(Clone)]
pub struct NonceDeDupEndpoint<S: Spec> {
    state: ApiState<S>,
}

impl<S: Spec> NonceDeDupEndpoint<S> {
    /// Creates a new `NonceDeDupEndpoint` instance.
    pub fn new(state: ApiState<S>) -> Self {
        Self { state }
    }
}

/// The response of the nonce module implementation.
#[derive(serde::Serialize, Clone)]
pub struct NonceResponse {
    /// The current nonce assiociated with the requested address.
    pub nonce: u64,
}

impl<S: Spec> DeDupEndpoint<S> for NonceDeDupEndpoint<S> {
    type Response = NonceResponse;

    type Error = anyhow::Error;

    fn handler(
        address: String,
        mut state: ApiStateAccessor<S>,
    ) -> Result<Self::Response, Self::Error> {
        let pub_key = <S::CryptoSpec as CryptoSpec>::PublicKey::from_str(&address)?;
        let credential_id = metered_credential(&pub_key, &mut state)?;
        let nonce = Uniqueness::<S>::default()
            .next_generation(&credential_id, &mut state)
            .unwrap();
        Ok(NonceResponse { nonce })
    }

    fn state(&self) -> ApiStateAccessor<S> {
        self.state.default_api_state_accessor()
    }
}
