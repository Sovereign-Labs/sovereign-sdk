use axum::routing::get;
use axum::Router;
use serde::Serialize;
use sov_modules_api::macros::config_value;
use sov_rest_utils::preconfigured_router_layers;

/// The response returned by the `/rollup/constants` endpoint.
///
/// For simplicity we currently only return a subset of constants that are useful
/// for clientside applications.
#[derive(Serialize)]
pub struct ConstantsResponse {
    chain_id: u64,
    chain_name: &'static str,
    hyperlane_domain: u32,
}

impl Default for ConstantsResponse {
    fn default() -> Self {
        Self {
            chain_id: config_value!("CHAIN_ID"),
            chain_name: config_value!("CHAIN_NAME"),
            hyperlane_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        }
    }
}

/// Returns an axum router configured to serve `/rollup/constants` requests
pub fn axum_router() -> Router<()> {
    preconfigured_router_layers(Router::new().route(
        "/rollup/constants",
        get(|| async move { axum::Json(ConstantsResponse::default()) }),
    ))
}
