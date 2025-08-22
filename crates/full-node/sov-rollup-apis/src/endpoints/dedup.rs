use std::str::FromStr;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use sov_modules_api::prelude::anyhow;
use sov_modules_api::rest::ApiState;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rest_utils::{errors, preconfigured_router_layers, Query};
use sov_rollup_interface::crypto::CredentialId;
use sov_uniqueness::Uniqueness;

/// Trait for the `/rollup/addresses/{credential_id}/dedup` endpoint.
///
/// Rollup developers should implement this to provide dedup functionality to external services
/// such as web3 SDK's in a generic way.
pub trait DeDupEndpoint<S: Spec>: Clone + Send + Sync + 'static {
    /// The response data returned by the `dedup` API endpoint.
    type Response: Serialize;

    /// An error that can be returned by the `dedup` endpoint.
    /// Will be included in the response body and can be used to debug failures in client-side
    /// applications.
    type Error: std::fmt::Display;

    /// Handle the `dedup` request.
    fn handler(
        credential_id: String,
        state: ApiStateAccessor<S>,
    ) -> Result<Self::Response, Self::Error>;

    /// Provides rollup state to the handler.
    fn state(&self) -> ApiStateAccessor<S>;

    /// Returns a configured axum router for the dedup endpoint.
    ///
    /// Calls the implemented [`DeDupEndpoint::handler`] and returns the result.
    /// If [`DeDupEndpoint::handler`] returns an error, then it will be included in the
    /// [`sov_rest_utils::ErrorObject`]s details field.
    ///
    /// # Warning
    ///
    /// If you override this method, you should ensure you provide the standard dedup path.
    /// If the path is different, then external tooling like web3 SDKs won't be able to consume the
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

/// Provides the `/rollup/addresses/{address}/dedup` endpoint using the sovereign provided
/// `uniqueness` module.
///
/// This endpoint supports two independent uniqueness mechanisms:
/// - **Nonces**: Sequential counters that must be used in order (0, 1, 2, ...). No skipping allowed.
/// - **Generations**: Non-sequential identifiers that can skip values (0, 3, 7, ...).
///
/// Both mechanisms are tracked independently per account, allowing flexible transaction ordering
/// strategies. You can mix nonce and generation-based transactions for the same account.
#[derive(Clone)]
pub struct SovereignDeDupEndpoint<S: Spec> {
    state: ApiState<S>,
}

impl<S: Spec> SovereignDeDupEndpoint<S> {
    /// Creates a new [`SovereignDeDupEndpoint`] instance.
    pub fn new(state: ApiState<S>) -> Self {
        Self { state }
    }
}

/// The response of the Dedup implementation for both nonce and generation number.
///
/// Only one field will be populated based on the query parameter:
/// - `nonce`: The next sequential nonce value (must be used in order)
/// - `generation`: The next available generation number (can skip values)
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DedupResponse {
    /// The next nonce associated with the requested address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    /// The next generation number associated with the requested address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

/// Query parameters for the dedup endpoint.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DedupQuery {
    /// Select which kind of uniqueness to return.
    /// If omitted, returns Nonce
    /// Example: `?select=nonce` or `?select=generation`
    pub select: Option<SelectField>,
}

/// Specifies which uniqueness field to return in the dedup response.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectField {
    /// Return the next nonce for the address.
    Nonce,
    /// Return the next generation number for the address.
    Generation,
}

impl<S: Spec> SovereignDeDupEndpoint<S> {
    fn handler_with_query(
        credential_id: String,
        mut state: ApiStateAccessor<S>,
        query: DedupQuery,
    ) -> Result<DedupResponse, anyhow::Error> {
        let credential_id = CredentialId::from_str(&credential_id)?;
        tracing::info!(%credential_id, "Going to provide dedup for");
        let uniqueness = Uniqueness::<S>::default();

        match query.select {
            Some(SelectField::Generation) => {
                let generation = uniqueness.next_generation(&credential_id, &mut state)?;
                tracing::info!(%credential_id, %generation, "Providing generation for credential id");
                Ok(DedupResponse {
                    nonce: None,
                    generation: Some(generation),
                })
            }
            Some(SelectField::Nonce) | None => {
                let nonce = uniqueness.next_nonce(&credential_id, &mut state)?;
                tracing::info!(%credential_id, %nonce, "Providing nonce for credential id");
                Ok(DedupResponse {
                    nonce: Some(nonce),
                    generation: None,
                })
            }
        }
    }
}

impl<S: Spec> DeDupEndpoint<S> for SovereignDeDupEndpoint<S> {
    type Response = DedupResponse;

    type Error = anyhow::Error;

    fn handler(
        credential_id: String,
        state: ApiStateAccessor<S>,
    ) -> Result<Self::Response, Self::Error> {
        Self::handler_with_query(credential_id, state, Default::default())
    }

    fn state(&self) -> ApiStateAccessor<S> {
        self.state.default_api_state_accessor()
    }

    fn axum_router(&self) -> Router<()> {
        preconfigured_router_layers(
            Router::new()
                .route(
                    "/rollup/addresses/:credential_id/dedup",
                    get(
                        |Path(credential_id): Path<String>,
                         State(state): State<Self>,
                         Query(query): Query<DedupQuery>| async move {
                            match Self::handler_with_query(credential_id, state.state(), query) {
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
