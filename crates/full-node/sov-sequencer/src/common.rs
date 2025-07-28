//! Defines the [`Sequencer`] trait and related types.

use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::StatusCode;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_blob_sender::{BlobExecutionStatus, BlobInternalId, BlobSenderHooks};
use sov_db::ledger_db::LedgerDb;
use sov_modules_api::capabilities::{AuthenticationOutput, TransactionAuthenticator};
use sov_modules_api::rest::utils::ErrorObject;
use sov_modules_api::rest::{ApiState, StateUpdateReceiver};
use sov_modules_api::*;
use sov_modules_stf_blueprint::{PreExecError, Runtime};
use sov_rest_utils::{json_obj, to_json_object};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::ledger_api::{ItemOrHash, LedgerStateProvider, QueryMode};
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use thiserror::Error;
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tokio::time::timeout;
use tracing::{info, trace};

use crate::rest_api::ApiAcceptedTx;
use crate::{SequencerNotReadyDetails, SlotNumber, TxHash, TxStatus, TxStatusManager};

pub(crate) type SequencerTxStream<Confirmation> =
    Pin<Box<dyn futures::Stream<Item = Result<ApiAcceptedTx<Confirmation>, anyhow::Error>> + Send>>;

pub(crate) type SequencerEventStream<Rt> = Pin<
    Box<
        dyn futures::Stream<
                Item = anyhow::Result<
                    RuntimeEventResponse<<Rt as RuntimeEventProcessor>::RuntimeEvent>,
                >,
            > + Send,
    >,
>;

/// The [`Sequencer`] trait is responsible for accepting transactions and
/// assembling them into batches.
#[async_trait]
pub trait Sequencer: Send + Sync + 'static {
    /// What data is returned to clients when a transaction is accepted.
    type Confirmation: Clone + serde::Serialize + Send + Sync + 'static;
    /// The rollup spec.
    type Spec: Spec;
    /// The rollup's [`Runtime`].
    type Rt: Runtime<Self::Spec>;
    /// The [`DaService`] used by the node (and sequencer).
    type Da: DaService<Spec = <Self::Spec as Spec>::Da>;

    /// Only available if the [`Sequencer`] supports events streaming.
    async fn subscribe_events(&self) -> Option<SequencerEventStream<Self::Rt>> {
        None
    }

    /// Only available if the [`Sequencer`] supports transactions streaming.
    async fn subscribe_transactions(
        &self,
        _starting_from: Option<u64>,
    ) -> Option<anyhow::Result<SequencerTxStream<Self::Confirmation>>> {
        None
    }

    /// Only available if the [`Sequencer`] blob streaming.
    async fn subscribe_blobs_from_blob_sender(
        &self,
    ) -> Option<broadcast::Receiver<BlobExecutionStatus<<Self::Da as DaService>::Spec>>> {
        None
    }

    /// Get a list of events by event numbers.
    ///
    /// This is only available if the [`Sequencer`] supports events streaming.
    async fn list_events(
        &self,
        _event_numbers: std::ops::Range<u64>,
    ) -> Result<
        Vec<RuntimeEventResponse<<Self::Rt as RuntimeEventProcessor>::RuntimeEvent>>,
        anyhow::Error,
    > {
        Ok(vec![])
    }

    /// Returns an [`ApiState`] subscribed to updates of the batch builder's state.
    fn api_state(&self) -> ApiState<Self::Spec>;

    /// Checks whether the batch builder is ready to accept transactions.
    async fn is_ready(&self) -> Result<(), SequencerNotReadyDetails>;

    /// Queries a transaction's status.
    async fn tx_status(
        &self,
        tx_hash: &TxHash,
    ) -> anyhow::Result<TxStatus<<<Self::Spec as Spec>::Da as DaSpec>::TransactionId>>;

    /// Queries a transaction by hash.
    async fn get_tx(
        &self,
        _tx_hash: TxHash,
    ) -> anyhow::Result<Option<AcceptedTx<Self::Confirmation>>> {
        Ok(None)
    }

    /// Updates the sequencer's view of the state of the rollup.
    async fn update_state(
        &self,
        update_info: StateUpdateInfo<<Self::Spec as Spec>::Storage>,
    ) -> anyhow::Result<()>;

    /// Adds a **not-encoded** transaction to the mempool. The [`Sequencer`]
    /// implementation itself is responsible for "encoding" the transaction.
    ///
    /// Can return an error if transaction is invalid or mempool is full.
    async fn accept_tx(
        &self,
        tx: FullyBakedTx,
    ) -> Result<AcceptedTx<Self::Confirmation>, ErrorObject>;

    /// Can be used to query and update the status of transactions.
    fn tx_status_manager(&self) -> &TxStatusManager<<Self::Spec as Spec>::Da>;

    /// Closes the current batch.
    async fn force_close_current_batch(&self) {
        panic!("Not implemented")
    }

    /// Subscribe to state update completion notifications. Note that notifications may be delivered out of order.
    async fn subscribe_state_updates_unstable(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<StateUpdateNotification>> {
        None
    }
}

/// A transaction that has been accepted by the batch builder.
#[serde_with::serde_as]
#[derive(Clone, serde::Serialize, derivative::Derivative)]
#[derivative(Debug)]
pub struct AcceptedTx<C> {
    /// Encoded transaction, as will appear on-chain.
    #[serde_as(as = "serde_with::base64::Base64")]
    pub tx: FullyBakedTx,
    /// Hash of the transaction.
    pub tx_hash: TxHash,
    #[derivative(Debug(bound = "C: Debug"))]
    /// Confirmation data. Could be empty, a receipt, or other data.
    pub confirmation: C,
}

/// A notification that the sequencer has processed a new state update.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StateUpdateNotification {
    /// The slot number that was processed.
    pub slot_number: SlotNumber,
    /// The finalized slot number.
    pub finalized_slot_number: SlotNumber,
}

impl<C> AcceptedTx<C> {
    /// Maps the inner confirmation data.
    pub fn map_confirmation<D>(self, f: impl FnOnce(C) -> D) -> AcceptedTx<D> {
        AcceptedTx {
            tx: self.tx,
            tx_hash: self.tx_hash,
            confirmation: f(self.confirmation),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct WithCachedTxHashes<I> {
    pub inner: I,
    pub tx_hashes: Arc<Vec<TxHash>>,
}

/// Sends [`TxStatusManager`] notifications upon blob status changes.
pub struct TxStatusBlobSenderHooks<Da: DaSpec> {
    txsm: TxStatusManager<Da>,
    tx_hashes_by_blob_id: RwLock<HashMap<BlobInternalId, Arc<Vec<TxHash>>>>,
}

impl<Da: DaSpec> TxStatusBlobSenderHooks<Da> {
    pub fn new(txsm: TxStatusManager<Da>) -> Self {
        Self {
            txsm,
            tx_hashes_by_blob_id: Default::default(),
        }
    }

    pub async fn add_txs(&self, blob_id: BlobInternalId, tx_hashes: Arc<Vec<TxHash>>) {
        self.tx_hashes_by_blob_id
            .write()
            .await
            .insert(blob_id, tx_hashes);
    }

    async fn notify_all(&self, blob_id: BlobInternalId, status: TxStatus<Da::TransactionId>) {
        let tx_hashes = self
            .tx_hashes_by_blob_id
            .read()
            .await
            .get(&blob_id)
            .cloned()
            .unwrap_or_default();

        for tx_hash in &*tx_hashes {
            trace!(
                %tx_hash,
                ?status,
                %blob_id,
                "Notifying about transaction status as a result of blob submission",
            );
            self.txsm.notify(*tx_hash, status.clone());
        }
    }
}

#[async_trait]
impl<Da: DaSpec> BlobSenderHooks for TxStatusBlobSenderHooks<Da> {
    type Da = Da;

    async fn on_published_blob(
        &self,
        blob_id: BlobInternalId,
        _blob_hash: [u8; 32],
        da_tx_id: Da::TransactionId,
    ) {
        self.notify_all(blob_id, TxStatus::Published { da_tx_id })
            .await;
    }

    async fn on_finalized_blob(
        &self,
        blob_id: BlobInternalId,
        _blob_hash: [u8; 32],
        _da_tx_id: &<Self::Da as DaSpec>::TransactionId,
    ) {
        self.tx_hashes_by_blob_id.write().await.remove(&blob_id);
    }
}

/// Empty transaction confirmation data.
///
/// Serializes as an empty JSON object.
#[derive(Clone, serde::Serialize)]
pub struct EmptyConfirmation {}

type AuthRes<S, Rt, I> = (
    TxScratchpad<S, I>,
    Result<
        (
            AuthenticationOutput<S, <Rt as DispatchCall>::Decodable>,
            BasicGasMeter<S>,
        ),
        PreExecError,
    >,
);

/// Error types for state update polling
#[derive(Debug, Clone, Error, derive_more::Display)]
pub enum StateUpdateError {
    /// Shutdown was requested
    Shutdown,
}

/// Polls for the next state update, handling shutdown, timeout, and debug flags
pub async fn poll_state_update<S: Spec>(
    state_update_receiver: &mut StateUpdateReceiver<S::Storage>,
    shutdown_receiver: &watch::Receiver<()>,
    task_name: &'static str,
) -> Result<StateUpdateInfo<S::Storage>, StateUpdateError> {
    loop {
        let changed_with_timeout = timeout(
            std::time::Duration::from_secs(30),
            state_update_receiver.changed(),
        );

        let fut = future_or_shutdown(changed_with_timeout, shutdown_receiver);
        let FutureOrShutdownOutput::Output(timeout_result) = fut.await else {
            return Err(StateUpdateError::Shutdown);
        };

        match timeout_result {
            Ok(changed_result) => {
                if changed_result.is_err() {
                    tracing::error!("State update sender was dropped. The sequencer can no longer receive state updates - shutting it down. This is a bug, please report it.");
                    return Err(StateUpdateError::Shutdown);
                }

                let info = (*state_update_receiver.borrow()).clone();

                return Ok(info);
            }
            Err(_) => {
                // Timeout elapsed. Log a warning then continue waiting. TODO: handle excessive timeouts (#2938).
                tracing::warn!(
                    task_name,
                    "Timeout waiting for state update notification, continuing to wait"
                );
            }
        }
    }
}

/// Does something -anything- in a loop every time the [`StateUpdateReceiver`]
/// receives a new value. A new trigger is generated with a timeout even if no
/// value is received.
///
/// Automagically handles shutdown and error checking for you.
pub async fn react_to_state_updates<S, Fut>(
    mut state_update_receiver: StateUpdateReceiver<S::Storage>,
    shutdown_receiver: watch::Receiver<()>,
    task_name: &'static str,
    mut closure: impl FnMut(StateUpdateInfo<S::Storage>) -> Fut,
) where
    S: Spec,
    Fut: Future<Output = anyhow::Result<()>>,
{
    loop {
        let poll_result =
            poll_state_update::<S>(&mut state_update_receiver, &shutdown_receiver, task_name).await;

        match poll_result {
            Ok(info) => {
                if let Err(err) = closure(info).await {
                    tracing::error!(%err, task_name, "Error inside the sequencer background task's closure; this is a bug, please report it");
                    break;
                }
            }
            Err(StateUpdateError::Shutdown) => {
                info!(
                    task_name,
                    "Shutdown signal received, exiting sequencer background task"
                );
                break;
            }
        }
    }
}

pub async fn loop_call_update_state<Seq: Sequencer>(
    seq: Arc<Seq>,
    state_update_receiver: StateUpdateReceiver<<Seq::Spec as Spec>::Storage>,
    shutdown_receiver: watch::Receiver<()>,
) {
    react_to_state_updates::<Seq::Spec, _>(
        state_update_receiver,
        shutdown_receiver,
        "loop_call_update_state",
        |info| async {
            if cfg!(debug_assertions) {
                let skip_flag = std::env::var("SOV_TEST_PAUSE_SEQUENCER_UPDATE_STATE");
                if skip_flag == Ok("1".to_string()) {
                    tracing::warn!("skipping state update due to env var flag");
                    Ok(())
                } else {
                    seq.update_state(info).await
                }
            } else {
                seq.update_state(info).await
            }
        },
    )
    .await;
}

#[tracing::instrument(skip_all, level = "trace")]
pub async fn loop_send_tx_notifications<S: Spec, Rt: RuntimeEventProcessor>(
    state_update_receiver: StateUpdateReceiver<S::Storage>,
    shutdown_receiver: watch::Receiver<()>,
    ledger_db: &LedgerDb,
    txsm: &TxStatusManager<S::Da>,
) {
    // `Arc<Mutex<...>>` is, I suspect, overkill here. It's just a workaround
    // around the `FnMut` closure issues I was banging my head against while writing
    // this.
    let latest_processed_slot_number =
        Arc::new(Mutex::new(state_update_receiver.borrow().slot_number));

    react_to_state_updates::<S, _>(state_update_receiver, shutdown_receiver, "loop_send_tx_notifications", move |info| {
        let latest_processed_slot_number = latest_processed_slot_number.clone();
        async move {
            let storage_slot_number = info.slot_number;
            let range = latest_processed_slot_number.lock().await.range_inclusive(storage_slot_number);

            trace!(%storage_slot_number, "Querying slot data from node to notify about transaction status");

            for slot_number in range {
                let slot = ledger_db
                    .get_slot_by_number::<BatchSequencerReceipt<S>, TxReceiptContents<S>, RuntimeEventResponse<Rt::RuntimeEvent>>(
                        slot_number,
                        QueryMode::Full,
                    )
                    .await?
                    .expect("Received slot notification from node, but it's absent in the ledger. This is a bug, please report it");

                for batch in slot.batches.unwrap_or_default().iter() {
                    let ItemOrHash::Full(batch) = batch else {
                        continue;
                    };
                    for tx in batch.txs.as_deref().unwrap_or_default().iter() {
                        let ItemOrHash::Full(tx) = tx else {
                            continue;
                        };

                        let tx_hash = TxHash::new(tx.hash);
                        trace!(%tx_hash, "Notifying about transaction status as a result of node processing a slot");
                        txsm.notify(tx_hash, TxStatus::Processed);
                    }
                }
            }
            *latest_processed_slot_number.lock().await =info.slot_number;

            Ok(())
        }
    })
    .await;
}

pub fn pre_exec_err_to_accept_tx_err(err: PreExecError) -> ErrorObject {
    match err{
        PreExecError::SequencerError(error) => {
            ErrorObject {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "The sequencer is currently unavailable; contact the administrator if the problem persists".to_string(),
                details: json_obj!({
                    "error": error.to_string()
                })
            }

        },
        PreExecError::AuthError(error) => {
            // For certain kinds of authentication errors, 401
            // or 403 would be more appropriate. But we'd have
            // to inspect the error contents to determine the
            // most appropriate status code... so 400 will do.
            ErrorObject {
                status: StatusCode::BAD_REQUEST,
                message: "The transaction is invalid".to_string(),
                details: json_obj!({
                    "error": error.to_string()
                })
            }
        },
    }
}

pub fn generic_accept_tx_error(details: impl std::fmt::Debug) -> ErrorObject {
    ErrorObject {
        status: StatusCode::BAD_REQUEST,
        message: "The transaction is invalid".to_string(),
        details: json_obj!({
            "error": format!("{:?}", details)
        }),
    }
}

pub fn error_not_fully_synced(details: SequencerNotReadyDetails) -> ErrorObject {
    let summary = match details {
        SequencerNotReadyDetails::Syncing { .. } => "The node fell out of sync with the DA head, the sequencer is waiting to catch up. There may also be a small delay after the node has finished syncing.",
        SequencerNotReadyDetails::WaitingOnDa { .. } => {
            "The sequencer is waiting for the DA to finalize more blocks"
        }
        SequencerNotReadyDetails::WaitingOnBlobSender { .. } => {
            "The sequencer is waiting for the blob sender to be ready"
        }
        SequencerNotReadyDetails::PreferredSequencerRecovering => {
            // We have to manually construct the error because `PreferredSequencerRecovering` is not an object, so `to_json_object` panics
            return ErrorObject {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "The preferred sequencer is recovering from downtime and cannot provide soft-confirmations at this time; No new transactions can be accepted, try again later".to_string(),
                details: Default::default(),
            };
        }
        SequencerNotReadyDetails::Startup => {
            return ErrorObject {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "The sequencer is still initializing and is not yet ready to accept transactions.".to_string(),
                details: Default::default(),
            };
        }
        SequencerNotReadyDetails::PreferredSequencerAtStopHeight{
            height_to_stop_at,
            current_height,

        } => {
            return ErrorObject {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: format!("The preferred sequencer has reached the stop height {height_to_stop_at} and is no longer accepting transactions. Current height: {current_height}").to_string(),
                details: Default::default(),
            };
        }
        SequencerNotReadyDetails::ReplicaMode => {
            return ErrorObject {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "Sequencer is replica and cannot accept transactions".to_string(),
                details: Default::default(),
            };
        }
    };
    ErrorObject {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: format!("{summary}; No new transactions can be accepted, try again later"),
        details: to_json_object(details),
    }
}

pub fn tx_auth<S, Rt, I>(
    tx_scratchpad: TxScratchpad<S, I>,
    gas_price: <S::Gas as Gas>::Price,
    baked_tx: &FullyBakedTx,
) -> AuthRes<S, Rt, I>
where
    S: Spec,
    Rt: Runtime<S>,
    I: StateProvider<S>,
{
    let gas_meter = BasicGasMeter::new_with_gas(<S as GasSpec>::max_tx_check_costs(), gas_price);

    let mut pre_exec_ws = tx_scratchpad.to_pre_exec_working_set(gas_meter);

    let auth_res = match Rt::Auth::authenticate(baked_tx, &mut pre_exec_ws) {
        Ok(ok) => ok,
        Err(err) => {
            let tx_scratchpad = pre_exec_ws.to_scratchpad_and_gas_meter().0;
            return (tx_scratchpad, Err(PreExecError::AuthError(err)));
        }
    };

    let auth_res = (auth_res.0, auth_res.1, Rt::wrap_call(auth_res.2));
    let (tx_scratchpad, gas_meter) = pre_exec_ws.to_scratchpad_and_gas_meter();

    (tx_scratchpad, Ok((auth_res, gas_meter)))
}

/// Checks if the sender of a call is allowed to submit the message via the sequencer.
///
/// Returns true if either...
/// 1. The message is `sequencer_safe`, meaning that submitting will never change the sequencer's config
/// 2. The sender is an admin for this sequencer
pub fn sender_is_allowed<RT: Runtime<S>, S: Spec>(
    runtime: &RT,
    call: &<RT as DispatchCall>::Decodable,
    sender: &S::Address,
    sequencer_address: &<S::Da as DaSpec>::Address,
    admins: &[S::Address],
) -> bool {
    let destination_module = <RT as DispatchCall>::module_info(runtime, call.discriminant());
    destination_module.is_safe_for_sequencer(call.contents(), sequencer_address)
        || admins.contains(sender)
}
