//! This crate contains REST API implementations for Sovereign rollup endpoints.
//!
//! Provides functionality for rollup-specific endpoints including gas price queries,
//! sync status, transaction simulation, and other rollup operations.

#![deny(missing_docs)]

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use sov_modules_api::capabilities::{ChainState, HasCapabilities};
use sov_modules_api::prelude::anyhow;
use sov_modules_api::prelude::tokio::sync::watch;
use sov_modules_api::rest::StateUpdateReceiver;
use sov_modules_api::{Gas, Spec, StateCheckpoint, SyncStatus};
pub use sov_modules_stf_blueprint::ApplyTxResult;
use sov_modules_stf_blueprint::Runtime;
use sov_rest_utils::{errors, preconfigured_router_layers, ApiResult};

/// Provides functionality for various `/rollup` endpoints.
pub mod endpoints;

/// Router for rollup transaction-related endpoints.
///
/// Provides access to state updates and sync status for rollup endpoints.
#[derive(Clone)]
pub struct RollupTxRouter<S: Spec, R> {
    state_update_recv: StateUpdateReceiver<S::Storage>,
    sync_status_receiver: watch::Receiver<SyncStatus>,
    _phantom: std::marker::PhantomData<R>,
}

/// Container for gas price information returned by the `/rollup/base-fee-per-gas/latest` endpoint.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "S: Spec")]
pub struct GasPriceContainer<S: Spec> {
    base_fee_per_gas: <S::Gas as Gas>::Price,
}

/// Creates an axum router for rollup endpoints.
///
/// Returns a router configured with the following endpoints:
/// - `GET /rollup/base-fee-per-gas/latest` - Get the current base fee per gas
/// - `GET /rollup/sync-status` - Get the current sync status of the rollup
///
/// # Arguments
/// * `state_update_recv` - Receiver for state updates from the rollup
/// * `sync_status_receiver` - Receiver for sync status updates
///
/// # Returns
/// An axum [`Router`] configured with the rollup endpoints.
pub fn rollup_tx_router<S: Spec, R: Runtime<S> + HasCapabilities<S>>(
    state_update_recv: StateUpdateReceiver<S::Storage>,
    sync_status_receiver: watch::Receiver<SyncStatus>,
) -> axum::Router<()> {
    preconfigured_router_layers(
        axum::Router::new()
            .route(
                "/rollup/base-fee-per-gas/latest",
                get(get_latest_base_fee_per_gas::<S, R>),
            )
            .route("/rollup/sync-status", get(get_sync_status))
            .with_state(Arc::new(RollupTxRouter {
                state_update_recv,
                sync_status_receiver,
                _phantom: Default::default(),
            })),
    )
}

async fn get_sync_status<S: Spec, R: Runtime<S> + HasCapabilities<S>>(
    State(state): State<Arc<RollupTxRouter<S, R>>>,
) -> axum::Json<SyncStatus> {
    let sync_status = *state.sync_status_receiver.borrow();
    Json(sync_status)
}

async fn get_latest_base_fee_per_gas<S: Spec, R: Runtime<S> + HasCapabilities<S>>(
    State(state): State<Arc<RollupTxRouter<S, R>>>,
) -> ApiResult<GasPriceContainer<S>> {
    let storage = state.state_update_recv.borrow().storage.clone();
    let mut state_checkpoint = StateCheckpoint::new(storage, &R::default().kernel(), None);

    let base_fee_per_gas = R::default()
        .chain_state()
        .base_fee_per_gas(&mut state_checkpoint)
        .ok_or_else(|| {
            anyhow::anyhow!("Unable to get base fee per gas: slot may be too far in the future")
        })
        .map_err(errors::database_error_response_500)?;

    Ok(GasPriceContainer { base_fee_per_gas }.into())
}
