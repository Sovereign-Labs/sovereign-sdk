use axum::extract::State;
use axum::response::IntoResponse as _;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use sov_rest_utils::preconfigured_router_layers;

/// Trait for the `/rollup/constant` endpoint.
pub trait ConstantsEndpoint: Clone + Send + Sync + 'static {
    /// The response data returned by the `schema` endpoint.
    type Response: Serialize;

    /// Handles the `schema` request.
    /// Should return the [`Constants`] as a JSON object string.
    fn handler(&self) -> Self::Response;

    /// Returns a configured axum router for the schema endpoint.
    ///
    /// Calls the implemented [`ConstantsEndpoint::handler`] and returns the result.
    /// If [`ConstantsEndpoint::handler`] returns a error then it will be included in the
    /// [`sov_rest_utils::ErrorObject`]s details field.
    ///
    /// # Warning
    ///
    /// If you override this method you should ensure you provide the standard schema path. If the
    /// path is different then external tooling like web3 SDKs won't be able to consume the
    /// functionality and will fail to work.
    fn axum_router(&self) -> Router<()> {
        preconfigured_router_layers(
            Router::new().route(
                "/rollup/constants",
                get(|State(state): State<Self>| async move {
                    axum::Json(state.handler()).into_response()
                })
                .with_state(self.clone()),
            ),
        )
    }
}

/// The response returned by the `/rollup/constants` endpoint.
///
/// For simplicity we currently only return a subset of constants that are useful
/// for clientside applications.
#[derive(Serialize, Clone)]
pub struct ConstantsResponse {
    /// the chain-id
    pub chain_id: u64,
    /// the chain-name
    pub chain_name: String,
    /// hyperlane domain for warp routes
    pub hyperlane_domain: u32,
    /// the address prefix
    pub address_prefix: &'static str,
}

impl ConstantsEndpoint for ConstantsResponse {
    type Response = Self;

    fn handler(&self) -> Self::Response {
        self.clone()
    }
}
