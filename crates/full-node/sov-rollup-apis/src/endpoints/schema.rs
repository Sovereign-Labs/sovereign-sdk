use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse as _;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use sov_modules_api::prelude::anyhow;
use sov_modules_api::prelude::tokio::sync::watch;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::{ConcurrentStateCheckpoint, HexHash, Spec};
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
    /// Should return the [`Schema`] as a JSON object string along with the chain hash as a hex
    /// string.
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

/// Standard implementation of the schema endpoint that dynamically resolves the chain hash
/// based on the current rollup height.
///
/// This endpoint uses chain hash overrides from the configuration to return the appropriate
/// chain hash for wallets. During chain hash transitions (including grace periods), this
/// ensures wallets always get the correct chain hash for signing transactions.
#[derive(Clone)]
pub struct StandardSchemaEndpoint<S: Spec> {
    schema: serde_json::Value,
    default_chain_hash: HexHash,
    checkpoint_receiver: watch::Receiver<Arc<ConcurrentStateCheckpoint<S>>>,
}

impl<S: Spec> StandardSchemaEndpoint<S> {
    /// Creates a new `StandardSchemaEndpoint`.
    ///
    /// # Arguments
    /// * `schema` - The schema to return
    /// * `default_chain_hash` - The default chain hash (from `Runtime::CHAIN_HASH`)
    /// * `checkpoint_receiver` - Receiver for state checkpoints to read current height
    pub fn new(
        schema: &Schema,
        default_chain_hash: HexHash,
        checkpoint_receiver: watch::Receiver<Arc<ConcurrentStateCheckpoint<S>>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            schema: serde_json::to_value(schema)?,
            default_chain_hash,
            checkpoint_receiver,
        })
    }
}

/// Response for the schema endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct StandardSchemaResponse {
    schema: serde_json::Value,
    chain_hash: HexHash,
}

impl<S: Spec> SchemaEndpoint for StandardSchemaEndpoint<S> {
    type Response = StandardSchemaResponse;

    type Error = anyhow::Error;

    fn handler(&self) -> Result<Self::Response, Self::Error> {
        // Get the current rollup height from the checkpoint
        let checkpoint = self.checkpoint_receiver.borrow();
        let height = checkpoint.rollup_height_to_access();

        // Resolve the chain hash for the current height
        let resolved = sov_modules_api::capabilities::resolve_chain_hashes_for_height(
            height.get(),
            self.default_chain_hash.0,
        );

        Ok(StandardSchemaResponse {
            schema: self.schema.clone(),
            chain_hash: resolved.primary.into(),
        })
    }
}
