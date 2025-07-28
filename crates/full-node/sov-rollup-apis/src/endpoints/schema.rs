use axum::extract::State;
use axum::response::IntoResponse as _;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use sov_modules_api::prelude::anyhow;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_rest_utils::{errors, preconfigured_router_layers};

/// Trait for the `/rollup/schema` endpoint.
///
/// This endpoint is utilized by client-side web3 SDK packages to provide the rollups current
/// schema. Providing the schema via a REST endpoint makes it easier to keep client-side libraries
/// in sync with the rollup version and detect when a new version has been deployed.
pub trait SchemaEndpoint: Clone + Send + Sync + 'static {
    /// The response data returned by the `schema` endpoint.
    type Response: Serialize;

    /// An error that will be returned by the `schema` endpoint.
    type Error: std::fmt::Display;

    /// Handles the `schema` request.
    /// Should return the [`Schema`] as a JSON object string.
    /// We return a `String` because [`Schema`] is not thread-safe or clonable even when wrapped in
    /// `Arc`.
    fn handler(&self) -> Result<Self::Response, Self::Error>;

    /// Returns a configured axum router for the schema endpoint.
    ///
    /// Calls the implemented [`SchemaEndpoint::handler`] and returns the result.
    /// If [`SchemaEndpoint::handler`] returns a error then it will be included in the
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
                "/rollup/schema",
                get(|State(state): State<Self>| async move {
                    match state.handler() {
                        Ok(data) => axum::Json(data).into_response(),
                        Err(err) => errors::bad_request_400("Failed to get rollup schema", err),
                    }
                })
                .with_state(self.clone()),
            ),
        )
    }
}

/// Provides a implementation of the `schema` endpoint using the schema JSON provided.
#[derive(Debug, Clone)]
pub struct StandardSchemaEndpoint {
    schema_json: String,
}

impl StandardSchemaEndpoint {
    /// Creates a new `StandardSchemaEndpoint` using the provided [`Schema`] as the JSON.
    pub fn new(schema: &Schema) -> anyhow::Result<Self> {
        Ok(Self {
            schema_json: serde_json::to_string(schema)?,
        })
    }
}

impl SchemaEndpoint for StandardSchemaEndpoint {
    type Response = serde_json::Value;

    type Error = anyhow::Error;

    fn handler(&self) -> Result<Self::Response, Self::Error> {
        Ok(serde_json::from_str(&self.schema_json)?)
    }
}
