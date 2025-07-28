//! This crate contains API specification to interact with the gas module.

#![deny(missing_docs)]

use axum::extract::State;
use axum::routing::{get, post};
use axum::Json;
use sov_api_spec::types::{self, SimulateExecutionResponse};
use sov_modules_api::capabilities::{AuthorizationData, HasCapabilities, UniquenessData};
use sov_modules_api::prelude::tokio::sync::watch;
use sov_modules_api::rest::StateUpdateReceiver;
use sov_modules_api::transaction::{Credentials, TxDetails};
use sov_modules_api::{CryptoSpec, DaSpec, Gas, PublicKey, Spec, SyncStatus};
pub use sov_modules_stf_blueprint::ApplyTxResult;
use sov_modules_stf_blueprint::Runtime;
use sov_rest_utils::{errors, preconfigured_router_layers, ApiResult};

mod client_interface;

mod default_provider;
/// Provides functionality for various `/rollup` endpoints.
pub mod endpoints;

pub use default_provider::DefaultRollupStateProvider;

/// Use [`RollupTxRouter::axum_router`] to instantiate an [`axum::Router`] for
/// a specific [`RollupStateProvider`].
#[derive(Clone)]
pub struct RollupTxRouter<T: RollupStateProvider> {
    state_update_recv: StateUpdateReceiver<<T::Spec as Spec>::Storage>,
    default_sequencer: <<T::Spec as Spec>::Da as DaSpec>::Address,
    default_sequencer_rollup_address: <T::Spec as Spec>::Address,
    sync_status_receiver: watch::Receiver<SyncStatus>,
}

/// The object returned by the `/base-fee-per-gas/latest` endpoint.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "S: Spec")]
pub struct GasPriceContainer<S: Spec> {
    base_fee_per_gas: <S::Gas as Gas>::Price,
}

/// The object returned by the `/rollup/simulate` endpoint.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "S: Spec")]
pub struct SimulateExecutionContainer<S: Spec> {
    /// The result of the simulation returned by the `apply_tx` method.
    pub apply_tx_result: ApplyTxResult<S>,
}

/// A partial transaction type that can be used to easily simulate the execution of a transaction.
/// This type is `partial` in the sense that it does not contain the full transaction details.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
#[serde(bound = "S: Spec")]
pub struct PartialTransaction<S: Spec> {
    /// The public key of the transaction sender.
    pub sender_pub_key: <S::CryptoSpec as CryptoSpec>::PublicKey,
    /// Call message encoded by a given runtime.
    pub encoded_call_message: Vec<u8>,
    /// The details of the transaction.
    pub details: TxDetails<S>,
    /// The generation of the transaction.
    pub generation: u64,
    /// An optional gas price for the transaction.
    /// If not provided, the current gas price will be used.
    pub gas_price: Option<<S::Gas as Gas>::Price>,
    /// The sequencer address of the transaction. If not provided, default sequencer address will be used.
    pub sequencer: Option<<S::Da as DaSpec>::Address>,
    /// The rollup address of the sequencer. If not provided, default sequencer rollup address will be used.
    pub sequencer_rollup_address: Option<S::Address>,
}

impl<S: Spec> From<PartialTransaction<S>> for AuthorizationData<S> {
    fn from(value: PartialTransaction<S>) -> AuthorizationData<S> {
        let pub_key = value.sender_pub_key.clone();
        let credential_id = pub_key.credential_id();
        let generation = value.generation;
        let default_address = credential_id.into();
        let credentials = Credentials::new(pub_key);
        // The generation module stores `raw_tx_hash`es, created from the full serialized tx
        // including the signature. Since we don't have the signature, we can't recreate this hash,
        // so we just use a value that will always pass the check. This makes the simulation
        // endpoint not report a failure IF you are sending duplicate transactions within the same
        // generation, so this is left as a responsibility of the user to avoid. (The assumption is
        // that normal users will not, in ordinary circumstances, send duplicate transactions
        // accidentally; so this is not the main purpose of the simulate endpoint anyway.)
        let tx_hash = ([0; 32]).into();

        AuthorizationData {
            uniqueness: UniquenessData::Generation(generation),
            tx_hash,
            credential_id,
            credentials,
            default_address,
        }
    }
}

/// A [`RollupStateProvider`] provides a way to query the state for information about gas.
pub trait RollupStateProvider: Clone + Send + Sync {
    /// The error type for fallible methods on this trait.
    type Error: ToString + Send + Sync + 'static;

    /// The [`Spec`] associated with the rollup state provider.
    type Spec: sov_modules_api::Spec;

    /// The [`Runtime`] associated with the rollup state provider.
    type Runtime: Runtime<Self::Spec>;

    /// Get the latest base fee per gas in the storage.
    fn get_latest_base_fee_per_gas(
        storage: &StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
    ) -> Result<<<Self::Spec as Spec>::Gas as Gas>::Price, Self::Error>;

    /// Simulates the execution of a transaction.
    fn simulate_execution(
        storage: &StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
        default_sequencer: <<Self::Spec as Spec>::Da as DaSpec>::Address,
        default_sequencer_rollup_address: <Self::Spec as Spec>::Address,
        transaction: PartialTransaction<Self::Spec>,
    ) -> Result<ApplyTxResult<Self::Spec>, Self::Error>;
}

impl<T> RollupTxRouter<T>
where
    T: RollupStateProvider + Clone + Send + Sync + 'static,
    T::Runtime: HasCapabilities<T::Spec>,
{
    /// Returns an [`axum::Router`] that exposes gas information, simulation data and sync status.
    pub fn axum_router(
        state_update_recv: StateUpdateReceiver<<T::Spec as Spec>::Storage>,
        default_sequencer: <<T::Spec as Spec>::Da as DaSpec>::Address,
        default_sequencer_rollup_address: <T::Spec as Spec>::Address,
        sync_status_receiver: watch::Receiver<SyncStatus>,
    ) -> axum::Router<()> {
        preconfigured_router_layers(
            axum::Router::new()
                .route(
                    "/rollup/base-fee-per-gas/latest",
                    get(Self::get_latest_base_fee_per_gas),
                )
                .route("/rollup/simulate", post(Self::simulate))
                .route("/rollup/sync-status", get(Self::get_sync_status))
                .with_state(RollupTxRouter {
                    state_update_recv,
                    default_sequencer,
                    default_sequencer_rollup_address,
                    sync_status_receiver,
                }),
        )
    }

    /// Handler for the `/rollup/sync-status` endpoint.
    async fn get_sync_status(
        State(RollupTxRouter {
            sync_status_receiver,
            ..
        }): State<Self>,
    ) -> axum::Json<SyncStatus> {
        let sync_status = *sync_status_receiver.borrow();
        Json(sync_status)
    }

    /// Get the latest base fee per gas in the storage.
    async fn get_latest_base_fee_per_gas(
        State(RollupTxRouter {
            state_update_recv, ..
        }): State<Self>,
    ) -> ApiResult<GasPriceContainer<T::Spec>> {
        match T::get_latest_base_fee_per_gas(&state_update_recv) {
            Ok(base_fee_per_gas) => Ok(GasPriceContainer { base_fee_per_gas }.into()),
            Err(err) => Err(errors::database_error_response_500(err)),
        }
    }

    /// Simulates the execution of a transaction
    async fn simulate(
        State(RollupTxRouter {
            state_update_recv,
            default_sequencer,
            default_sequencer_rollup_address,
            ..
        }): State<Self>,
        Json(req): Json<types::SimulateBody>,
    ) -> ApiResult<SimulateExecutionResponse> {
        let transaction: PartialTransaction<T::Spec> = req
            .body
            .try_into()
            .map_err(|err| errors::bad_request_400("Malformatted partial transaction", err))?;

        match T::simulate_execution(&state_update_recv, default_sequencer, default_sequencer_rollup_address, transaction) {
            Ok(apply_tx_result) =>
            {
                let simulate_execution_response: SimulateExecutionResponse = SimulateExecutionContainer { apply_tx_result }.try_into()
                    .map_err(|err| errors::internal_server_error_response_500(format!("Internal server error: Failed to serialize response. Error {err}")))?;

                Ok(simulate_execution_response.into())
            }

            Err(err) => Err(errors::bad_request_400(
                "The transaction simulation failed before the execution. Please check that the provided transaction details are correct",
                err,
            )),
        }
    }
}
