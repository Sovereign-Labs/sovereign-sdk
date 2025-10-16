//! Transaction simulation endpoints for the Sovereign rollup.
//!
//! This module provides REST API endpoints for simulating transaction execution
//! without actually committing them to the blockchain. This is useful for:
//! - Estimating gas costs before submitting transactions
//! - Testing transaction behavior and outcomes
//! - Debugging transaction failures
//! - Frontend applications showing transaction previews
use axum::http::StatusCode;
use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_modules_api::capabilities::{
    AuthorizationData, ChainState, TransactionAuthorizer, UniquenessData,
};
use sov_modules_api::common::Amount;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::anyhow;
use sov_modules_api::rest::StateUpdateReceiver;
use sov_modules_api::sov_universal_wallet::schema::{RollupRoots, SchemaError};
use sov_modules_api::transaction::{Credentials, PriorityFeeBips, TxDetails};
use sov_modules_api::{
    get_runtime_schema, AuthenticatedTransactionData, CredentialId, DaSpec, EventModuleName,
    FullyBakedTx, Gas, GasArray, HexHash, HexString, Runtime, Spec, StateCheckpoint,
    StateProvider as _, WorkingSet,
};
use sov_modules_stf_blueprint::{apply_tx, get_gas_used, ApplyTxResult};
use sov_rest_utils::{json_obj, preconfigured_router_layers, ErrorObject};
use sov_rollup_interface::stf::TxEffect;
use sov_uniqueness::Uniqueness;
use std::str::FromStr;

/// Trait for transaction simulation endpoints.
///
/// Implementations of this trait provide transaction simulation functionality
/// through a REST API. The simulation executes transactions against the current
/// blockchain state without committing the changes.
pub trait SimulateEndpoint: Send + Sync + 'static {
    /// The state used by the simulation router.
    type State: Clone + Send + Sync;

    /// The input parameters for the simulation request.
    type Parameters: DeserializeOwned + Clone + Send;

    /// The response type returned by successful simulations.
    type Response: Serialize;

    /// The error type for simulation failures.
    type Error: Into<ErrorObject>;

    /// Handles a simulation request.
    ///
    /// This method processes the simulation parameters and returns either
    /// a successful simulation result or an error describing what went wrong.
    ///
    /// # Arguments
    /// * `params` - The simulation parameters including transaction details
    ///
    /// # Returns
    /// * `Ok(Response)` - Successful simulation with gas usage and events
    /// * `Err(Error)` - Simulation failed due to invalid input or execution error
    fn handler(state: Self::State, params: Self::Parameters)
        -> Result<Self::Response, Self::Error>;

    /// Returns a configured axum router for the endpoint.
    ///
    /// Creates an axum router with the `/rollup/simulate` endpoint that accepts POST requests.
    /// The router calls the implemented [`Self::handler`] and returns the result as JSON.
    /// If [`Self::handler`] returns an error, it will be converted to an [`ErrorObject`] and
    /// returned with the appropriate HTTP status code.
    ///
    /// # Warning
    ///
    /// If you override this method, you should ensure you provide the standard `/rollup/simulate` path.
    /// If the path is different, then external tooling like web3 SDKs won't be able to consume the
    /// functionality and will fail to work.
    ///
    /// # Returns
    /// An axum [`Router`] configured with the simulation endpoint.
    fn axum_router(state: Self::State) -> Router<()> {
        preconfigured_router_layers(
            Router::new()
                .route(
                    "/rollup/simulate",
                    post(
                        |State(state): State<Self::State>, Json(body): Json<Self::Parameters>| async move {
                            match Self::handler(state, body) {
                                Ok(data) => Json(data).into_response(),
                                Err(err) => err.into().into_response(),
                            }
                        },
                    ),
                )
                .with_state(state.clone()),
        )
    }
}

/// Errors that can occur during transaction simulation.
#[derive(thiserror::Error, Debug)]
pub enum SimulateError {
    /// Invalid input parameters were provided.
    #[error("Invalid input")]
    InvalidInput(String),

    /// Failed to retrieve the current gas price from the chain state.
    #[error("Failed to retrieve gas price")]
    GasPriceRetrieval,

    /// Failed to resolve the transaction context for execution.
    #[error("Failed to resolve tx context")]
    ContextResolution(#[source] anyhow::Error),

    /// Failed to create the runtime schema needed for call deserialization.
    #[error("Failed to create schema for rollup")]
    SchemaConstruction(#[source] anyhow::Error),

    /// Failed to serialize the call message from JSON to bytes.
    #[error("Failed to serialize call message")]
    CallSerialization(#[from] SchemaError),

    /// Failed to decode the call bytes into a runtime call.
    #[error("Failed to decode call message into a runtime call")]
    CallDecoding(#[from] std::io::Error),
}

impl From<SimulateError> for ErrorObject {
    fn from(value: SimulateError) -> Self {
        let (status, error) = match &value {
            SimulateError::InvalidInput(message) => (StatusCode::BAD_REQUEST, message.clone()),
            SimulateError::GasPriceRetrieval => (StatusCode::INTERNAL_SERVER_ERROR, "".to_string()),
            SimulateError::ContextResolution(error) | SimulateError::SchemaConstruction(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
            SimulateError::CallSerialization(error) => (StatusCode::BAD_REQUEST, error.to_string()),
            SimulateError::CallDecoding(error) => {
                // Internal server error because if JSON to bytes serialization works
                // then bytes to RuntimeCall should also work.
                (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
        };

        Self {
            status,
            message: value.to_string(),
            details: json_obj!({"error":error}),
        }
    }
}

const NULL_TX_HASH: HexHash = HexString([0; 32]);

/// Sequencer configuration for transaction simulation.
///
/// Contains the addresses needed to simulate transactions in the context
/// of a specific sequencer.
#[derive(Clone)]
pub struct SequencerSimulate<S: Spec> {
    rollup_address: S::Address,
    da_address: <S::Da as DaSpec>::Address,
}

/// Main implementation of the simulation endpoint for Sovereign rollups.
///
/// This struct provides transaction simulation functionality by executing
/// transactions against the current state without committing changes.
/// It supports gas estimation, event emission, and error handling.
#[derive(Clone)]
pub struct SovereignSimulate<S: Spec, R> {
    state_receiver: StateUpdateReceiver<S::Storage>,
    default_sequencer: SequencerSimulate<S>,
    _phantom: std::marker::PhantomData<R>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SimulatedEvent<E> {
    value: E,
    key: String,
    module: String,
}

/// Successful simulation outcome containing gas usage and events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessOutcome<E> {
    /// The amount of gas consumed by the transaction.
    gas_used: Amount,
    /// The priority fee reward of the transaction expressed as a gas token amount.
    priority_fee: Amount,
    /// Events emitted during transaction execution.
    events: Vec<SimulatedEvent<E>>,
}

/// Failed simulation outcome with error reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailOutcome {
    /// The reason why the transaction failed.
    pub reason: String,
}

/// The outcome of a transaction simulation.
///
/// This enum represents the three possible outcomes when simulating a transaction:
/// successful execution, transaction revert, or transaction skipped due to errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SimulateOutcome<E> {
    /// The transaction executed successfully.
    Success(SuccessOutcome<E>),
    /// The transaction was reverted during execution.
    Reverted(FailOutcome),
    /// The transaction was skipped due to pre-execution errors.
    Skipped(FailOutcome),
}

impl<S: Spec, R: Runtime<S>> SovereignSimulate<S, R> {
    /// Creates a new simulation endpoint instance.
    ///
    /// # Arguments
    /// * `receiver` - State update receiver for accessing current blockchain state
    /// * `sequencer_rollup_address` - Default rollup address for the sequencer
    /// * `sequencer_da_address` - Default data availability address for the sequencer
    ///
    /// # Returns
    /// A new `SovereignSimulate` instance ready to handle simulation requests.
    pub fn new(
        receiver: StateUpdateReceiver<S::Storage>,
        sequencer_rollup_address: S::Address,
        sequencer_da_address: <S::Da as DaSpec>::Address,
    ) -> Self {
        Self {
            state_receiver: receiver,
            default_sequencer: SequencerSimulate {
                rollup_address: sequencer_rollup_address,
                da_address: sequencer_da_address,
            },
            _phantom: Default::default(),
        }
    }

    /// Converts this simulation endpoint into an axum router.
    ///
    /// This is a convenience method that wraps the endpoint in an `Arc` and
    /// creates the router using the `SimulateEndpoint` trait implementation.
    /// The resulting router can be merged with other routers or used directly
    /// to serve the simulation endpoint.
    ///
    /// # Returns
    /// An axum [`Router`] configured with the `/rollup/simulate` endpoint.
    pub fn into_router(self) -> Router<()> {
        Self::axum_router(std::sync::Arc::new(self))
    }

    fn tx_details(&self, partial: TxDetailsParameter) -> Result<TxDetails<S>, SimulateError> {
        let gas_limit = partial
            .gas_limit
            .map(TryInto::try_into)
            .transpose()
            .map_err(|e| SimulateError::InvalidInput(format!("{e:?}")))?;
        Ok(TxDetails {
            chain_id: config_value!("CHAIN_ID"),
            max_priority_fee_bips: partial
                .max_priority_fee_bips
                .unwrap_or(PriorityFeeBips::ZERO),
            max_fee: partial.max_fee.unwrap_or(Amount::MAX),
            gas_limit,
        })
    }

    fn sequencer(
        &self,
        partial: SequencerParameter,
    ) -> Result<SequencerSimulate<S>, SimulateError> {
        let rollup_address: S::Address = if let Some(input) = partial.rollup_address {
            S::Address::from_str(&input).map_err(|_| {
                SimulateError::InvalidInput("failed to parse sequencer rollup address".to_owned())
            })?
        } else {
            self.default_sequencer.rollup_address.clone()
        };
        let da_address = if let Some(input) = partial.da_address {
            <S::Da as DaSpec>::Address::from_str(&input).map_err(|_| {
                SimulateError::InvalidInput("failed to parse sequencer da address".to_owned())
            })?
        } else {
            self.default_sequencer.da_address.clone()
        };
        Ok({
            SequencerSimulate {
                rollup_address,
                da_address,
            }
        })
    }

    fn authorization_data(
        &self,
        params: &SimulateParameters,
        state: &mut StateCheckpoint<S>,
    ) -> AuthorizationData<S> {
        let credential_id = CredentialId::from_str(&params.sender).unwrap();
        let uniqueness = params.uniqueness.unwrap_or_else(|| {
            let generation = Uniqueness::<S>::default()
                .next_generation(&credential_id, state)
                .unwrap();
            UniquenessData::Generation(generation)
        });

        AuthorizationData {
            tx_hash: NULL_TX_HASH,
            uniqueness,
            credential_id,
            default_address: credential_id.into(),
            credentials: Credentials::new(credential_id),
        }
    }

    fn outcome(&self, result: ApplyTxResult<S>) -> SimulateOutcome<R::RuntimeEvent> {
        let gas_price = result.transaction_consumption.gas_price();
        let gas_used = get_gas_used(&result.receipt);
        let events = result
            .receipt
            .events
            .iter()
            .map(|stored_event| {
                // This should be infailable, a transaction execution should never
                // produce events that aren't (de)serializable by the runtime
                let value: R::RuntimeEvent = borsh::de::BorshDeserialize::try_from_slice(
                    stored_event.value().inner().as_slice(),
                )
                .unwrap();
                let key = String::from_utf8(stored_event.key().inner().clone())
                    .unwrap_or_else(|_| hex::encode(stored_event.key().inner()));
                let module = value.module_name().to_string();
                SimulatedEvent { value, key, module }
            })
            .collect::<Vec<_>>();

        match result.receipt.receipt {
            TxEffect::Skipped(e) => SimulateOutcome::Skipped(FailOutcome {
                reason: e.error.to_string(),
            }),
            TxEffect::Reverted(e) => SimulateOutcome::Reverted(FailOutcome {
                reason: e.reason.to_string(),
            }),
            TxEffect::Successful(_) => SimulateOutcome::Success(SuccessOutcome {
                priority_fee: result.transaction_consumption.priority_fee().0,
                gas_used: gas_used.value(gas_price),
                events,
            }),
        }
    }
}

/// Optional transaction details for simulation requests.
///
/// These parameters allow customizing the transaction execution environment
/// for simulation purposes.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TxDetailsParameter {
    /// Maximum priority fee in basis points.
    pub max_priority_fee_bips: Option<PriorityFeeBips>,
    /// Maximum fee the sender is willing to pay.
    pub max_fee: Option<Amount>,
    /// Gas limit for the transaction as an array of u64 values.
    pub gas_limit: Option<Vec<u64>>,
}

/// Optional sequencer configuration for simulation requests.
///
/// Allows overriding the default sequencer addresses for simulation purposes.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SequencerParameter {
    /// Custom rollup address for the sequencer.
    pub rollup_address: Option<String>,
    /// Custom data availability address for the sequencer.
    pub da_address: Option<String>,
}

/// Parameters for transaction simulation requests.
///
/// This struct contains all the information needed to simulate a transaction
/// including the sender, the call to execute, and optional configuration.
#[derive(Serialize, Deserialize, Clone)]
pub struct SimulateParameters {
    /// The credential ID of the transaction sender as a string.
    pub sender: String,
    /// The runtime call to simulate, provided as JSON.
    pub call: serde_json::Value,
    /// Optional transaction details like gas limits and fees.
    pub tx_details: Option<TxDetailsParameter>,
    /// Optional sequencer configuration overrides.
    pub sequencer: Option<SequencerParameter>,
    /// Optional uniqueness data for the transaction.
    /// If not provided a valid uniqueness will be used.
    pub uniqueness: Option<UniquenessData>,
}

impl<S: Spec, R: Runtime<S>> SimulateEndpoint for SovereignSimulate<S, R> {
    type State = std::sync::Arc<Self>;

    type Parameters = SimulateParameters;

    type Response = SimulateOutcome<R::RuntimeEvent>;

    type Error = SimulateError;

    fn handler(
        state: Self::State,
        params: Self::Parameters,
    ) -> Result<Self::Response, Self::Error> {
        let mut runtime = R::default();
        let mut accessor = StateCheckpoint::new(
            state.state_receiver.borrow().storage.clone(),
            &runtime.kernel(),
        );
        let gas_price = runtime
            .chain_state()
            .base_fee_per_gas(&mut accessor)
            .ok_or(SimulateError::GasPriceRetrieval)?;
        let auth_data = state.authorization_data(&params, &mut accessor);
        let sequencer = state.sequencer(params.sequencer.unwrap_or_default())?;
        let auth_tx_data =
            AuthenticatedTransactionData(state.tx_details(params.tx_details.unwrap_or_default())?);

        let mut scratchpad = accessor.to_tx_scratchpad();
        let context = runtime
            .transaction_authorizer()
            .resolve_context(
                &auth_data,
                &sequencer.da_address,
                sequencer.rollup_address,
                &mut scratchpad,
            )
            .map_err(SimulateError::ContextResolution)?;

        let ws_gas_meter = auth_tx_data.gas_meter(gas_price, <S::Gas>::MAX);
        let working_set = WorkingSet::create_working_set(scratchpad, &auth_tx_data, ws_gas_meter);

        let schema = get_runtime_schema::<S, R>().map_err(SimulateError::SchemaConstruction)?;
        let call_bytes = schema.json_to_borsh(
            schema.rollup_expected_index(RollupRoots::RuntimeCall)?,
            &params.call.to_string(),
        )?;
        let runtime_call = R::decode_call(&call_bytes)?;

        let (result, _) = apply_tx(
            &mut runtime,
            &context,
            &auth_tx_data,
            // We don't have a way to get the raw transaction hash because it depends on the signature.
            NULL_TX_HASH,
            // We use an empty tx body because we can't build an actual one without the signature. We could make something *more* accurate here, but it might be confusing to the user.
            FullyBakedTx::new(vec![]),
            runtime_call,
            working_set,
        );

        Ok(state.outcome(result))
    }
}
