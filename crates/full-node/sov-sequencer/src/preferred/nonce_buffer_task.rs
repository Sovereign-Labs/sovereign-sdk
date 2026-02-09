use async_trait::async_trait;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::rest::ApiState;
use sov_modules_api::{
    FullyBakedTx, Gas, Runtime, SkippedTxContents, Spec, TransactionReceipt, TxProcessingError,
};
use sov_rollup_interface::{
    crypto::CredentialId,
    node::{future_or_shutdown, FutureOrShutdownOutput},
    TxHash,
};
use std::cmp::Ordering as CmpOrdering;
use std::collections::btree_map;
use std::collections::hash_map;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::common::{AcceptedTx, ForcedTxBatchNotification};
use crate::metrics::{
    MetricBatcher, NonceBufferMainQueueBlockedMetric, NonceBufferMainQueueDepthMetric,
    NonceBufferTimeoutQueueMetric,
};
use crate::preferred::rate_limiter::IpAndCredentialId;

use super::block_executor::RollupBlockExecutorError;
use super::sync_sequencer_state::{
    AcceptTxError, DoNewTxError, SequencerStateUpdator, SequencerStateUpdatorError,
};
use super::Confirmation;

// At 20_000, we should be able to cover 2s at 10k TPS if they're all out of order nonce
// submissions.
// Note that the nonce queue is completely independent of max batch size, so it's possible to spam
// it to the limit with enough effort. This may have consequences for rollups that expect slow but
// large TXs. Perhaps we should consider enforcing batch size limits before the nonce check in accept_tx()?
const MAX_BUFFERED_TXS: usize = 20_000;

// Maximum depth of the main input queue for the nonce buffer task.
// This is separate from MAX_BUFFERED_TXS because the main input queue handles more than just
// buffered transactions (e.g., TxExecuted, TxPersisted messages). Currently set to the same
// value pending real-world testing.
const MAX_BUFFER_INPUT_QUEUE: usize = 20_000;

// Batches metrics together for performance instead of sending them every single message.
const METRICS_BATCH_SIZE: usize = 32;

/// Send a message to an mpsc channel, returning how long the send was blocked.
///
/// Uses try_send first for the non-blocking fast path. If the channel is full, falls back to
/// blocking send and records how long the send was blocked.
///
/// Returns Ok(blocked_time_us) on success (0 if not blocked), or Err if the channel is closed.
async fn send_and_measure_block_time<T>(
    sender: &mpsc::Sender<T>,
    message: T,
    trace_name: &'static str,
) -> Result<u64, mpsc::error::SendError<T>> {
    match sender.try_send(message) {
        Ok(()) => Ok(0),
        Err(mpsc::error::TrySendError::Full(message)) => {
            tracing::trace!("{trace_name} is full. Blocking until capacity available.");
            let started_blocking = Instant::now();
            sender.send(message).await?;
            Ok(started_blocking.elapsed().as_micros() as u64)
        }
        Err(mpsc::error::TrySendError::Closed(message)) => Err(mpsc::error::SendError(message)),
    }
}

/// Get the current depth of an mpsc channel from its sender.
fn queue_depth<T>(sender: &mpsc::Sender<T>) -> usize {
    sender.max_capacity() - sender.capacity()
}

pub(crate) type TransactionExecutorResult<S, Rt> =
    Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>;

pub(crate) type TransactionReceiverResult<S, Rt> =
    Result<TransactionExecutorResult<S, Rt>, SequencerStateUpdatorError>;

/// Request to schedule a timeout for a queued transaction.
/// Sent from the main nonce buffer task to the dedicated timeout queue task.
struct TimeoutRequest {
    credential_id: CredentialId,
    tx_nonce: u64,
    tx_hash: TxHash,
    queued_at: Instant,
}

struct QueuedTx<S: Spec, Rt: Runtime<S>> {
    pub baked_tx: FullyBakedTx,
    pub tx_hash: TxHash,
    pub ip_addr_and_credential: IpAndCredentialId<S::Address>,
    pub original_tx_queue_id: u64,
    pub nonce_when_queued: u64,
    pub queued_at: Instant,
    pub result_sender: oneshot::Sender<TransactionReceiverResult<S, Rt>>,
}

/// Tracks the nonce for transactions which have been executed through the queue, but haven't been
/// persisted to database (and therefore to state) yet.
#[derive(Debug, Default)]
struct NonPersistedTxs {
    /// The last transaction that completed execution _and_ was successful.
    last_successfully_executed: Option<u64>,
    /// Whether we know of a transaction with the current nonce, whose execution outcome is as of
    /// yet unknown. E.g. it's currently mid-execution, or it was popped off of the head of the
    /// queue for execution.
    has_in_flight: bool,
}

impl NonPersistedTxs {
    /// If this is Some(), this nonce can be used in place of the state nonce.
    /// I.e. a transaction with this nonce can be executed immediately, a transaction with an
    /// earlier nonce can be rejected.
    fn user_nonce(&self) -> Option<u64> {
        self.last_successfully_executed
            .map(|n| n.checked_add(1).expect("Overflow adding 1 to user nonce"))
    }

    /// If this is Some(), this nonce can be used as the starting point for pre-requisite checks.
    /// I.e. if the queue contains contiguous transactions following this nonce up to some nonce N,
    /// then transactions up to N should not be evicted on timeout.
    fn user_nonce_to_use_as_prerequisite_start(&self) -> Option<u64> {
        match self.has_in_flight {
            true => Some(
                self.user_nonce()
                    .unwrap_or(0)
                    .checked_add(1)
                    .expect("Overflow adding 1 to user nonce"),
            ),
            false => self.user_nonce(),
        }
    }

    fn mark_inflight(&mut self, tx_nonce_check: u64) {
        if self
            .user_nonce()
            .is_some_and(|user_nonce| tx_nonce_check != user_nonce)
        {
            let msg = "Sequencer nonce buffer: non-persisted tracking: attempted to execute nonce that does not match tracked next nonce!";
            tracing::error!(msg);
            // Once this code has been battle-tested in production for a bit, the debug_assert can
            // be upgraded to an assert and the if-statement elided entirely to simplify.
            debug_assert!(false, "{msg}");
        } else if self.last_successfully_executed.is_none() {
            // If we're marking a transction as in-flight, that means we definitely know the
            // previous one has been executed. Probably from the API state.
            self.last_successfully_executed = tx_nonce_check.checked_sub(1);
        }
        // We don't throw an error if has_in_flight is already true - for instance, if two txs with
        // the current valid nonce arrive near-simultaneously, we let the executor sort them out for
        // simplicity. Thus there can be more than one tx in flight - they just all need to have
        // the same nonce.
        // See the comment on mark_inflight_execution_succeeded for an explanation of how this
        // could be avoided.
        self.has_in_flight = true;
    }

    fn mark_inflight_execution_succeeded(&mut self, tx_nonce_check: u64) {
        // We don't throw an error if has_in_flight is already false, because two transactions with
        // the same nonce arriving at the same time will both be queued (letting the STF sort them
        // out) thus their inflight/not-inflight markings will be interleaved.
        // This is mostly fine though it creates a small window of time where, if the first
        // transaction is rejected, prerequisite checks will erroneously fail after the first
        // transaction has completed but before the second one has. This could be fixed by
        // reworking the same-nonce logic to actually queue transactions that match the currently
        // in-flight nonce; and then after receivint a TxExecuted event, evicting it on success or
        // queueing it (with a NewTx) on failure. The latter logic is already mostly in-place; all
        // that needs to be added is the eviction logic, and then `assert!(self.has_in_flight)` can
        // be re-added here.
        //
        // However since pre-requisite checks have been disabled for now, this hasn't been fully
        // implemented and tested yet.
        self.has_in_flight = false;

        self.last_successfully_executed = self.last_successfully_executed.map(|last| {
            let new = last.checked_add(1).expect("Overflow adding 1 to user nonce");
            if new != tx_nonce_check {
                let msg = "Sequencer nonce buffer: non-persisted tracking: after executing, incremented nonce did not match tx nonce!";
                tracing::error!(msg);
                debug_assert!(false, "{msg}"); // See comment in mark_inflight
            }
            new
        }).or(Some(tx_nonce_check));
    }

    fn mark_inflight_execution_failed(&mut self) {
        // See the comment in mark_inflight_execution_succeeded: if the queueing behaviour for
        // multiple TXs with the currently valid nonce is improved, we could assert that
        // has_in_flight == true here. But right now this invariant doesn't hold.
        self.has_in_flight = false;
    }
}

#[derive(Default)]
struct AddressQueue<S: Spec, Rt: Runtime<S>> {
    txs: BTreeMap<u64, QueuedTx<S, Rt>>,
    non_persisted: NonPersistedTxs,
}

impl<S: Spec, Rt: Runtime<S>> AddressQueue<S, Rt> {
    #[allow(dead_code)]
    fn has_contiguity_between(&self, starting_nonce: u64, tx_nonce: u64) -> bool {
        let mut expected = starting_nonce;
        for &nonce in self.txs.keys() {
            if nonce >= tx_nonce {
                break;
            }
            if nonce != expected {
                return false; // Gap found
            }
            expected += 1;
        }

        // Return true if we've reached tx_nonce, meaning we have all prerequisites
        expected >= tx_nonce
    }
}

#[derive(derivative::Derivative)]
#[derivative(Debug(bound = "S: Spec, Rt: Runtime<S>"))]
enum NonceBufferInput<S: Spec, Rt: Runtime<S>> {
    /// A tx to be executed. Either newly arrived from the API, or newly valid and popped from the
    /// head of the queue.
    NewTx {
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        ip_addr_and_credential: IpAndCredentialId<S::Address>,
        tx_nonce: u64,
        original_tx_queue_id: u64,
        result_sender: oneshot::Sender<TransactionReceiverResult<S, Rt>>,
    },
    /// Tx execution finished; the results will be reported to the API handler. If needed, the next
    /// tx will be popped from the queue.
    TxExecuted {
        credential_id: CredentialId,
        tx_nonce: u64,
        tx_result: TransactionReceiverResult<S, Rt>,
        result_sender: oneshot::Sender<TransactionReceiverResult<S, Rt>>,
    },
    // A user's tx was persisted to the databse. If the user's queue is empty and its non_persisted
    // nonces have been persisted to state, the queue may now be cleaned up.
    TxPersisted {
        credential_id: CredentialId,
        tx_nonce: u64,
    },
    // Tx has been waiting in the queue until the timeout has hit. If the pre-requisite nonces have
    // not been queued yet, it may get evicted now.
    TxTimedOut {
        credential_id: CredentialId,
        tx_nonce: u64,
        tx_hash: TxHash,
    },
}

/// Time out transactions using an mpsc channel as a FIFO queue.
struct TimeoutQueueTask<S: Spec, Rt: Runtime<S>> {
    /// Receives timeout requests from the main nonce buffer task.
    input: mpsc::Receiver<TimeoutRequest>,
    /// Sends TxTimedOut notifications back to the main nonce buffer task.
    output: mpsc::Sender<NonceBufferInput<S, Rt>>,
    /// The timeout duration for all transactions.
    timeout_duration: Duration,
    /// Shutdown notification.
    shutdown_receiver: watch::Receiver<()>,
}

impl<S: Spec, Rt: Runtime<S>> TimeoutQueueTask<S, Rt> {
    async fn run(mut self) {
        loop {
            let req = match future_or_shutdown(self.input.recv(), &self.shutdown_receiver).await {
                FutureOrShutdownOutput::Shutdown | FutureOrShutdownOutput::Output(None) => return,
                FutureOrShutdownOutput::Output(Some(req)) => req,
            };
            let elapsed = req.queued_at.elapsed();
            let remaining = self.timeout_duration.saturating_sub(elapsed);

            // Sleep until timeout is due
            if !remaining.is_zero() {
                match future_or_shutdown(tokio::time::sleep(remaining), &self.shutdown_receiver)
                    .await
                {
                    FutureOrShutdownOutput::Shutdown => return,
                    FutureOrShutdownOutput::Output(()) => {}
                }
            }

            // Send timeout notification to main task
            // If the channel is closed (main task shut down), exit the loop
            if self
                .output
                .send(NonceBufferInput::TxTimedOut {
                    credential_id: req.credential_id,
                    tx_nonce: req.tx_nonce,
                    tx_hash: req.tx_hash,
                })
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

/// This trait encodes the functionality that the buffer task needs for handling submission of
/// queued transactions.
#[async_trait]
pub trait TxExecutionBackend<S: Spec, Rt: Runtime<S>>: Clone {
    fn get_current_nonce_for_user(&self, credential_id: &CredentialId) -> u64;
    fn get_current_executor_tx_queue_id(&self) -> u64;
    async fn execute_tx(
        &self,
        baked_tx: &FullyBakedTx,
        tx_hash: TxHash,
        ip_addr_and_credential: IpAndCredentialId<S::Address>,
        original_tx_queue_id: u64,
        reason: &'static str,
    ) -> TransactionReceiverResult<S, Rt>;
}

/// The Sequencer backend is a standard implementation when the nonce buffer is used in the
/// preferred sequencer, and allows the sequencer to delegate executing transactions for submission
/// to the queue.
pub struct SequencerTxExecutionBackend<S: Spec, Rt: Runtime<S>> {
    pub api_state: ApiState<S>,
    pub executor_queue_id: Arc<AtomicU64>,
    pub state_updator: Arc<SequencerStateUpdator<S, Rt>>,
}

impl<S: Spec, Rt: Runtime<S>> Clone for SequencerTxExecutionBackend<S, Rt> {
    fn clone(&self) -> Self {
        Self {
            api_state: self.api_state.clone(),
            executor_queue_id: self.executor_queue_id.clone(),
            state_updator: self.state_updator.clone(),
        }
    }
}

#[async_trait]
impl<S: Spec, Rt: Runtime<S>> TxExecutionBackend<S, Rt> for SequencerTxExecutionBackend<S, Rt> {
    fn get_current_nonce_for_user(&self, credential_id: &CredentialId) -> u64 {
        let mut state = self.api_state.default_api_state_accessor();
        sov_uniqueness::Uniqueness::<S>::default()
            .nonce(credential_id, &mut state)
            .unwrap_infallible()
            .unwrap_or_default()
    }

    fn get_current_executor_tx_queue_id(&self) -> u64 {
        self.executor_queue_id.load(AtomicOrdering::Acquire)
    }

    async fn execute_tx(
        &self,
        baked_tx: &FullyBakedTx,
        tx_hash: TxHash,
        ip_addr_and_credential: IpAndCredentialId<S::Address>,
        original_tx_queue_id: u64,
        reason: &'static str,
    ) -> TransactionReceiverResult<S, Rt> {
        self.state_updator
            .accept_tx_msg(
                baked_tx,
                tx_hash,
                original_tx_queue_id,
                ip_addr_and_credential,
                reason,
            )
            .await
    }
}

#[derive(derivative::Derivative)]
#[derivative(Clone(bound = ""))]
pub struct NonceBufferInputSender<E: TxExecutionBackend<S, Rt>, S: Spec, Rt: Runtime<S>> {
    buffer_sender_channel: mpsc::Sender<NonceBufferInput<S, Rt>>,
    execution_backend: E,
    shutdown_receiver: watch::Receiver<()>,
}

pub struct NonceBufferTask<E: TxExecutionBackend<S, Rt>, S: Spec, Rt: Runtime<S>> {
    buffers: HashMap<CredentialId, AddressQueue<S, Rt>>,
    buffer_input: mpsc::Receiver<NonceBufferInput<S, Rt>>,
    forced_tx_batch_receiver: broadcast::Receiver<ForcedTxBatchNotification>,
    input_sender: NonceBufferInputSender<E, S, Rt>,
    execution_backend: E,
    maximum_future_nonce_delta: u64,
    timeout_sender: mpsc::Sender<TimeoutRequest>,
    /// Batches metrics for the timeout queue (sender side, combined blocked time + depth).
    timeout_metrics_batcher: MetricBatcher<NonceBufferTimeoutQueueMetric>,
    /// Batches metrics for main queue depth (receiver side, only depth - block time is tracked by sender).
    main_queue_depth_batcher: MetricBatcher<NonceBufferMainQueueDepthMetric>,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Execute,
    Enqueue,
    Reject,
}

fn determine_action(tx_nonce: u64, user_nonce: u64, maximum_future_nonce_delta: u64) -> Action {
    let max_nonce_to_queue = user_nonce
        .checked_add(maximum_future_nonce_delta)
        .expect("Overflow adding maximum_future_nonce_delta to user nonce");
    if tx_nonce > user_nonce && tx_nonce <= max_nonce_to_queue {
        Action::Enqueue
    } else if tx_nonce == user_nonce {
        Action::Execute
    } else {
        Action::Reject
    }
}

impl<E: TxExecutionBackend<S, Rt> + Clone + Send + Sync + 'static, S: Spec, Rt: Runtime<S>>
    NonceBufferTask<E, S, Rt>
{
    /// Schedule a timeout for a queued transaction.
    /// Sends to the timeout queue with metrics tracking.
    async fn schedule_timeout(
        &mut self,
        credential_id: CredentialId,
        tx_nonce: u64,
        tx_hash: TxHash,
    ) {
        let request = TimeoutRequest {
            credential_id,
            tx_nonce,
            tx_hash,
            queued_at: Instant::now(),
        };

        match send_and_measure_block_time(
            &self.timeout_sender,
            request,
            "Nonce buffer timeout queue",
        )
        .await
        {
            Ok(blocked_for_us) => {
                self.timeout_metrics_batcher
                    .push(NonceBufferTimeoutQueueMetric {
                        blocked_for_us,
                        queue_depth: queue_depth(&self.timeout_sender),
                    });
            }
            Err(_) => {
                // Flush metrics on shutdown since we might not get another chance
                self.timeout_metrics_batcher.flush();
                tracing::warn!(
                    "Nonce buffer timeout channel closed while scheduling timeout. \
                     Shutdown likely in progress. tx_hash={tx_hash}"
                );
            }
        }
    }

    fn drain_and_reject_all_txs(&mut self, reject_error: fn() -> TransactionReceiverResult<S, Rt>) {
        while let Ok(input) = self.buffer_input.try_recv() {
            match input {
                NonceBufferInput::NewTx { result_sender, .. } => {
                    let _ = result_sender.send(reject_error());
                }
                NonceBufferInput::TxExecuted {
                    tx_result,
                    result_sender,
                    ..
                } => {
                    let _ = result_sender.send(tx_result);
                }
                NonceBufferInput::TxTimedOut {
                    credential_id,
                    tx_nonce,
                    tx_hash,
                } => self.handle_tx_timed_out(credential_id, tx_nonce, tx_hash),
                NonceBufferInput::TxPersisted { .. } => (),
            }
        }

        let buffers = std::mem::take(&mut self.buffers);
        for (_credential_id, queue) in buffers {
            for (_nonce, tx) in queue.txs {
                let _ = tx.result_sender.send(reject_error());
            }
        }

        // Flush metrics since we might not get another chance to report them.
        self.timeout_metrics_batcher.flush();
        self.main_queue_depth_batcher.flush();
    }

    fn handle_tx_timed_out(&mut self, credential_id: CredentialId, tx_nonce: u64, tx_hash: TxHash) {
        let queue = self.buffers.entry(credential_id).or_default();
        let user_nonce_for_prerequisites = queue
            .non_persisted
            .user_nonce_to_use_as_prerequisite_start()
            .unwrap_or_else(|| {
                self.execution_backend
                    .get_current_nonce_for_user(&credential_id)
            });
        // Pre-requisite checks have been disabled to simplify.
        // See the comments in the methods on NonPersisted for extra improvements on
        // transactions with identical nonces that will make pre-requisite checks work
        // reliably; additionally a time bound on execution would be needed (e.g. retry
        // limit).
        //
        // if queue.has_contiguity_between(user_nonce_for_prerequisites, tx_nonce) {
        //     self.schedule_timeout(credential_id, tx_nonce, tx_hash);
        // }
        match queue.txs.entry(tx_nonce) {
            // If the hash doesn't match, it means the tx has been replaced. The
            // `result_sender` is for the new tx and we shouldn't notify it.
            btree_map::Entry::Occupied(entry) if entry.get().tx_hash == tx_hash => {
                let tx = entry.remove();
                let _ = tx.result_sender.send(err_invalid_nonce::<S, Rt>(
                    tx_hash,
                    tx_nonce,
                    user_nonce_for_prerequisites,
                    tx.nonce_when_queued,
                    tx.queued_at,
                    credential_id,
                    InvalidNonceReason::Timeout,
                ));
            }
            _ => (),
        }
    }

    async fn run(&mut self, shutdown_receiver: &mut watch::Receiver<()>) {
        let mut input_closed = false;
        let mut wipe_closed = false;
        loop {
            if input_closed && wipe_closed {
                return;
            }
            tokio::select! {
                // If shutdown and another branch are both ready, prioritize shutdown so queued
                // messages are drained with shutdown semantics deterministically.
                biased;
                _ = shutdown_receiver.changed() => {
                    tracing::info!("Nonce buffer task shutting down. Rejecting queued transactions.");
                    self.drain_and_reject_all_txs(shutdown_reject_error::<S, Rt>);
                    return;
                }
                input = self.buffer_input.recv(), if !input_closed => {
                    match input {
                        Some(input) => {
                            // Track main queue depth on receive side with batching
                            self.main_queue_depth_batcher
                                .push(NonceBufferMainQueueDepthMetric {
                                    queue_depth: queue_depth(&self.input_sender.buffer_sender_channel),
                                });

                            match input {
                                NonceBufferInput::NewTx {
                                    baked_tx,
                                    tx_hash,
                                    ip_addr_and_credential,
                                    tx_nonce,
                                    original_tx_queue_id,
                                    result_sender,
                                } => {
                                    let queue = self
                                        .buffers
                                        .entry(ip_addr_and_credential.credential_id)
                                        .or_default();

                                    let user_nonce = queue.non_persisted.user_nonce().unwrap_or_else(|| {
                                        self.execution_backend
                                            .get_current_nonce_for_user(&ip_addr_and_credential.credential_id)
                                    });

                                    match determine_action(tx_nonce, user_nonce, self.maximum_future_nonce_delta) {
                                        Action::Enqueue => {
                                            // Replacement handling: if a tx with the same nonce is already in the
                                            // queue...
                                            //  * If it's the same tx (same hash): we reject the new request
                                            //  * If it was a different tx: we replace it with the new one, and
                                            //  send a rejection to the old one
                                            if let btree_map::Entry::Occupied(old_entry) = queue.txs.entry(tx_nonce)
                                            {
                                                let old_tx = old_entry.get();
                                                if old_tx.tx_hash == tx_hash {
                                                    let _ = result_sender.send(err_invalid_nonce::<S, Rt>(
                                                        tx_hash,
                                                        tx_nonce,
                                                        user_nonce,
                                                        old_tx.nonce_when_queued,
                                                        old_tx.queued_at,
                                                        ip_addr_and_credential.credential_id,
                                                        InvalidNonceReason::AlreadyQueued,
                                                    ));
                                                    continue;
                                                } else {
                                                    let old_tx = old_entry.remove();
                                                    let _ = old_tx.result_sender.send(err_invalid_nonce::<S, Rt>(
                                                        old_tx.tx_hash,
                                                        tx_nonce,
                                                        user_nonce,
                                                        old_tx.nonce_when_queued,
                                                        old_tx.queued_at,
                                                        ip_addr_and_credential.credential_id,
                                                        InvalidNonceReason::Replaced,
                                                    ));
                                                }
                                            }
                                            queue.txs.insert(
                                                tx_nonce,
                                                QueuedTx {
                                                    baked_tx,
                                                    tx_hash,
                                                    ip_addr_and_credential,
                                                    original_tx_queue_id,
                                                    nonce_when_queued: user_nonce,
                                                    queued_at: Instant::now(),
                                                    result_sender,
                                                },
                                            );
                                            self.schedule_timeout(
                                                ip_addr_and_credential.credential_id,
                                                tx_nonce,
                                                tx_hash,
                                            )
                                            .await;
                                        }
                                        Action::Execute => {
                                            // The executor queue ID is incremented whenever the sequencer has
                                            // downtime.
                                            // It invalidates all pre-downtime transactions. So we need to empty
                                            // the nonce queue too.
                                            if self.execution_backend.get_current_executor_tx_queue_id()
                                                > original_tx_queue_id
                                            {
                                                let _ = result_sender.send(wipe_reject_error());
                                                self.drain_and_reject_all_txs(wipe_reject_error::<S, Rt>);
                                                continue;
                                            }
                                            queue.non_persisted.mark_inflight(tx_nonce);
                                            let input_sender = self.input_sender.clone();
                                            let backend = self.execution_backend.clone();
                                            tokio::spawn(async move {
                                                let tx_result = backend
                                                    .execute_tx(
                                                        &baked_tx,
                                                        tx_hash,
                                                        ip_addr_and_credential,
                                                        original_tx_queue_id,
                                                        "nonce_queue_immediate",
                                                    )
                                                    .await;
                                                let send_result = input_sender
                                                    .send_to_main_queue(NonceBufferInput::TxExecuted {
                                                        credential_id: ip_addr_and_credential.credential_id,
                                                        tx_nonce,
                                                        tx_result,
                                                        result_sender,
                                                    })
                                                    .await;
                                                // If the main loop has already exited (e.g., shutdown),
                                                // return the execution result directly to the caller.
                                                if let Err(mpsc::error::SendError(
                                                    NonceBufferInput::TxExecuted {
                                                        tx_result,
                                                        result_sender,
                                                        ..
                                                    },
                                                )) = send_result
                                                {
                                                    let _ = result_sender.send(tx_result);
                                                }
                                            });
                                        }
                                        Action::Reject => {
                                            let _ = result_sender.send(err_invalid_nonce::<S, Rt>(
                                                tx_hash,
                                                tx_nonce,
                                                user_nonce,
                                                user_nonce,
                                                Instant::now(),
                                                ip_addr_and_credential.credential_id,
                                                InvalidNonceReason::Invalid,
                                            ));
                                        }
                                    }
                                }
                                NonceBufferInput::TxExecuted {
                                    credential_id,
                                    tx_nonce,
                                    tx_result,
                                    result_sender,
                                } => {
                                    // If the tx was rejected because the sequencer is down (syncing,
                                    // recovering etc.), the nonce queue will be stale and needs to be wiped.
                                    // At minimum the non_persisted tracking for all users needs wiping for
                                    // correctness, but pre-downtime transactions are invalidated by the
                                    // sequencer anyway so we wipe everything.
                                    if is_notready_error(&tx_result) {
                                        let _ = result_sender.send(tx_result);
                                        self.drain_and_reject_all_txs(wipe_reject_error::<S, Rt>);
                                        continue;
                                    }

                                    let queue = self.buffers.entry(credential_id).or_default();
                                    if tx_result.as_ref().is_ok_and(|r| r.is_ok()) {
                                        queue
                                            .non_persisted
                                            .mark_inflight_execution_succeeded(tx_nonce);
                                    } else {
                                        queue.non_persisted.mark_inflight_execution_failed();
                                    }

                                    let _ = result_sender.send(tx_result); // If the receiver was dropped, ignore

                                    let user_nonce = queue.non_persisted.user_nonce().unwrap_or_else(|| {
                                        self.execution_backend
                                            .get_current_nonce_for_user(&credential_id)
                                    });
                                    loop {
                                        let Some(head_entry) = queue.txs.first_entry() else {
                                            break;
                                        };
                                        match head_entry.key().cmp(&user_nonce) {
                                            CmpOrdering::Less => {
                                                // Stale transaction in queue - evict and ignore.
                                                // This should not normally happen either, but we handle it to avoid a deadlock
                                                // if it does happen for any reason.
                                                let msg = format!("The nonce buffer task evicted a stale transaction for user {} with nonce {}; the user's current nonce is believed to be {user_nonce}. Stale transactions should not exist in the nonce buffer. The user will see an EvictedBeforeExecution error.", credential_id, head_entry.key());
                                                head_entry.remove();
                                                tracing::error!(msg);
                                                debug_assert!(false, "{msg}");
                                                continue;
                                            }
                                            CmpOrdering::Greater => {
                                                // First transaction starts in the future - nothing ready to execute yet
                                                break;
                                            }
                                            CmpOrdering::Equal => {
                                                // First transaction is the next expected nonce. Pop it and send
                                                // as a NewTx.
                                                let tx = head_entry.remove();
                                                queue.non_persisted.mark_inflight(user_nonce);
                                                let _ = self
                                                    .input_sender
                                                    .send_to_main_queue(NonceBufferInput::NewTx {
                                                        baked_tx: tx.baked_tx,
                                                        tx_hash: tx.tx_hash,
                                                        ip_addr_and_credential: tx.ip_addr_and_credential,
                                                        tx_nonce: user_nonce,
                                                        original_tx_queue_id: tx.original_tx_queue_id,
                                                        result_sender: tx.result_sender,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                NonceBufferInput::TxPersisted {
                                    credential_id,
                                    tx_nonce,
                                } => {
                                    let hash_map::Entry::Occupied(entry) = self.buffers.entry(credential_id) else {
                                        continue;
                                    };
                                    if entry.get().txs.is_empty()
                                        && entry
                                            .get()
                                            .non_persisted
                                            .user_nonce_to_use_as_prerequisite_start()
                                            .is_none_or(|n| n <= tx_nonce)
                                    {
                                        entry.remove();
                                    }
                                }
                                NonceBufferInput::TxTimedOut {
                                    credential_id,
                                    tx_nonce,
                                    tx_hash,
                                } => self.handle_tx_timed_out(credential_id, tx_nonce, tx_hash),
                            }
                        }
                        None => {
                            input_closed = true;
                        }
                    }
                }
                forced_tx_batch = self.forced_tx_batch_receiver.recv(), if !wipe_closed => {
                    match forced_tx_batch {
                        Ok(notification) => {
                            tracing::info!(?notification, "Wiping nonce buffer after forced batch execution");
                            self.drain_and_reject_all_txs(wipe_reject_error::<S, Rt>);
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(skipped, "Forced batch notifications lagged; wiping nonce buffer");
                            self.drain_and_reject_all_txs(wipe_reject_error::<S, Rt>);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            wipe_closed = true;
                        }
                    }
                }
            }
        }
    }

    pub fn spawn(
        execution_backend: E,
        maximum_future_nonce_delta: u64,
        future_nonce_transaction_timeout_millis: u64,
        forced_tx_batch_receiver: broadcast::Receiver<ForcedTxBatchNotification>,
        mut shutdown_receiver: watch::Receiver<()>,
    ) -> (JoinHandle<()>, NonceBufferInputSender<E, S, Rt>) {
        let (buffer_sender_channel, buffer_input) = mpsc::channel(MAX_BUFFER_INPUT_QUEUE);
        let (timeout_sender, timeout_receiver) = mpsc::channel(MAX_BUFFERED_TXS);

        let timeout_task = TimeoutQueueTask {
            input: timeout_receiver,
            output: buffer_sender_channel.clone(),
            timeout_duration: Duration::from_millis(future_nonce_transaction_timeout_millis),
            shutdown_receiver: shutdown_receiver.clone(),
        };

        let input_sender = NonceBufferInputSender {
            buffer_sender_channel,
            execution_backend: execution_backend.clone(),
            shutdown_receiver: shutdown_receiver.clone(),
        };

        let mut main_task = NonceBufferTask {
            buffers: Default::default(),
            buffer_input,
            forced_tx_batch_receiver,
            input_sender: input_sender.clone(),
            execution_backend,
            maximum_future_nonce_delta,
            timeout_sender,
            timeout_metrics_batcher: MetricBatcher::new(METRICS_BATCH_SIZE),
            main_queue_depth_batcher: MetricBatcher::new(METRICS_BATCH_SIZE),
        };

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = main_task.run(&mut shutdown_receiver) => {}
                _ = timeout_task.run() => {}
            }
        });
        (handle, input_sender)
    }
}

impl<E: TxExecutionBackend<S, Rt> + Send + 'static, S: Spec, Rt: Runtime<S>>
    NonceBufferInputSender<E, S, Rt>
{
    fn is_shutting_down(&self) -> bool {
        self.shutdown_receiver.has_changed().unwrap_or(true)
    }

    /// Send a message to the main input queue with metrics tracking.
    /// Uses try_send first, falling back to blocking send if the channel is full.
    /// Returns Err if the channel is closed (shutdown in progress).
    async fn send_to_main_queue(
        &self,
        input: NonceBufferInput<S, Rt>,
    ) -> Result<(), mpsc::error::SendError<NonceBufferInput<S, Rt>>> {
        let blocked_for_us = send_and_measure_block_time(
            &self.buffer_sender_channel,
            input,
            "Nonce buffer main queue",
        )
        .await?;

        // On the sender side, only submit a metric when we were blocked.
        // This should happen pretty seldom, so we won't be spamming telegraf too much.
        // Queue depth is tracked on the receiver side with batching.
        if blocked_for_us > 0 {
            sov_metrics::track_metrics(|t| {
                t.submit(NonceBufferMainQueueBlockedMetric { blocked_for_us });
            });
        }
        Ok(())
    }

    pub async fn handle_new_tx(
        &self,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        tx_nonce: u64,
        ip_addr_and_credential: IpAndCredentialId<S::Address>,
        original_tx_queue_id: u64,
    ) -> TransactionReceiverResult<S, Rt> {
        let (queue_sender, queue_receiver) = oneshot::channel();
        self.send_to_main_queue(NonceBufferInput::NewTx {
            baked_tx,
            tx_hash,
            ip_addr_and_credential,
            tx_nonce,
            original_tx_queue_id,
            result_sender: queue_sender,
        })
        .await
        .map_err(|e| {
            tracing::warn!("Sequencer nonce buffer input receiver dropped. Assuming sequencer shutdown. Error: {e:?}");
            SequencerStateUpdatorError::Shutdown
        })?;
        let nonce_when_queued = self
            .execution_backend
            .get_current_nonce_for_user(&ip_addr_and_credential.credential_id);
        let queued_at = Instant::now();
        queue_receiver.await.unwrap_or_else(|_| {
            // The oneshot sender was dropped. This should normally only happen
            // on shutdown, or if there's a bug.
            if self.is_shutting_down() {
                return shutdown_reject_error::<S, Rt>();
            }
            let current_nonce = self
                .execution_backend
                .get_current_nonce_for_user(&ip_addr_and_credential.credential_id);
            err_invalid_nonce::<S, Rt>(
                tx_hash,
                tx_nonce,
                current_nonce,
                nonce_when_queued,
                queued_at,
                ip_addr_and_credential.credential_id,
                InvalidNonceReason::EvictedBeforeExecution,
            )
        })
    }

    pub async fn mark_tx_persisted(&self, credential_id: CredentialId, tx_nonce: u64) {
        let _ = self
            .send_to_main_queue(NonceBufferInput::TxPersisted {
                credential_id,
                tx_nonce,
            })
            .await;
    }
}

/// Helper enum for error logging when the nonce queue rejects a transaction.
#[derive(Clone, Debug)]
enum InvalidNonceReason {
    /// Nonce is in the past or beyond the queue limit. Tx rejected immediately when received.
    Invalid,
    /// Tx was queued but the timeout was reached before pre-requisites were all received, so tx
    /// was evicted from the queue.
    Timeout,
    /// Tx was queued but executor oneshot was dropped. This happens on sequencer shutdown OR if
    /// the queue enters an inconsistent state and internally evicts stale transactions (which
    /// should almost never happen in practice unless the queue has a bug).
    EvictedBeforeExecution,
    /// The nonce was queued for execution, but a new transaction with the same nonce arrived
    /// before it could be executed.
    Replaced,
    /// The same transaction (with the same nonce AND hash) is already in the queue.
    AlreadyQueued,
}

/// Helper function to create an invalid nonce error
fn err_invalid_nonce<S: Spec, Rt: Runtime<S>>(
    tx_hash: TxHash,
    tx_nonce: u64,
    expected_nonce: u64,
    nonce_when_queued: u64,
    instant_queued: std::time::Instant,
    credential_id: CredentialId,
    queue_rejection_reason: InvalidNonceReason,
) -> TransactionReceiverResult<S, Rt> {
    let was_queued_msg = format!("The sequencer queued the transaction for reordering {} ms ago, when the user's nonce was {nonce_when_queued}.", instant_queued.elapsed().as_millis());
    let queue_error_msg = match queue_rejection_reason {
        InvalidNonceReason::Invalid => "The sequencer did not attempt to queue the transaction as it was not within valid queue limits (either in the past, or beyond the max limit).".to_string(),
        InvalidNonceReason::Timeout => format!("{was_queued_msg} In that time, the sequencer did not accept all the transactions leading up to this tx's nonce, so it has timed out and is being evicted from the queue."),
        InvalidNonceReason::EvictedBeforeExecution => format!("{was_queued_msg} It was now dropped from the queue for an unknown reason. This should normally only happen when the sequencer is shutting down."),        
        InvalidNonceReason::Replaced => format!("{was_queued_msg} But a new transaction with the same nonce has arrived and replaced it in the account's queue."),
        InvalidNonceReason::AlreadyQueued => format!("An identical transaction with the same hash is already in the nonce queue; its existing status is unchanged from this request. {was_queued_msg}"),
    };
    let error_msg = format!(
        "Tx bad nonce for credential id: {credential_id}, expected: {expected_nonce}, but found: {tx_nonce}. {queue_error_msg}"
    );
    tracing::debug!(
        "Sequencer rejecting nonce transaction with error: {error_msg}, tx_hash: {tx_hash}"
    );
    let receipt = TransactionReceipt {
        tx_hash,
        body_to_save: None,
        events: Vec::new(),
        receipt: sov_rollup_interface::stf::TxEffect::Skipped(SkippedTxContents {
            gas_used: <S::Gas as Gas>::zero(),
            error: TxProcessingError::CheckUniquenessFailed(error_msg),
        }),
    };
    Ok(Err(AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
        RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
    ))))
}

fn wipe_reject_error<S: Spec, Rt: Runtime<S>>() -> TransactionReceiverResult<S, Rt> {
    Ok(Err(AcceptTxError::SequencerOverloaded503))
}

fn shutdown_reject_error<S: Spec, Rt: Runtime<S>>() -> TransactionReceiverResult<S, Rt> {
    Err(SequencerStateUpdatorError::Shutdown)
}

fn is_notready_error<S: Spec, Rt: Runtime<S>>(result: &TransactionReceiverResult<S, Rt>) -> bool {
    match result {
        Err(_) => false,
        Ok(receiver_result) => match receiver_result {
            Ok(_) => false,
            Err(accept_tx_error) => match accept_tx_error {
                AcceptTxError::NotFullySynced(_) => true,
                AcceptTxError::ReplicaMode
                | AcceptTxError::SequencerOverloaded503
                | AcceptTxError::BatchError { .. }
                | AcceptTxError::NewTxError(_)
                | AcceptTxError::RateLimiter(_) => false,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferred::{DoNewTxError, RollupBlockExecutorError};
    use crate::SequencerNotReadyDetails;
    use sov_modules_api::{SkippedTxContents, TransactionReceipt, TxProcessingError};
    use sov_test_utils::runtime::TestOptimisticRuntime;
    use sov_test_utils::TestSpec;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::oneshot;

    type TestRuntime = TestOptimisticRuntime<TestSpec>;

    const DEFAULT_TEST_MAX_QUEUE_SIZE: u64 = 100;
    const DEFAULT_TEST_QUEUE_TIMEOUT_MS: u64 = 100; // 100ms timeout should be enough for sync txs to execute without delaying unit tests much

    // Helper to create a mock QueuedTx for testing
    fn create_mock_queued_tx(nonce: u8) -> QueuedTx<TestSpec, TestRuntime> {
        create_mock_queued_tx_with_hash(nonce, [nonce; 32])
    }

    fn create_mock_queued_tx_with_hash(
        nonce: u8,
        hash: [u8; 32],
    ) -> QueuedTx<TestSpec, TestRuntime> {
        let (sender, _receiver) = oneshot::channel();
        QueuedTx {
            baked_tx: FullyBakedTx {
                // Hacky fake data to help track nonces when TXs are sent through the queue
                data: vec![nonce].into(),
                sequencing_data: None,
            },
            tx_hash: TxHash::from(hash),
            ip_addr_and_credential: IpAndCredentialId {
                address: <TestSpec as Spec>::Address::from([1; 28]),
                credential_id: CredentialId::from([1u8; 32]),
                ip_addr: std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
            },
            nonce_when_queued: 0,
            queued_at: Instant::now(),
            original_tx_queue_id: 0,
            result_sender: sender,
        }
    }

    #[test]
    fn test_has_contiguous_sequence_empty_queue() {
        let queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::default();

        // Empty queue should return false when target > current_nonce
        assert!(!queue.has_contiguity_between(0, 5));

        // But if target == current_nonce, return true even on empty queue (no prerequisites needed)
        assert!(queue.has_contiguity_between(0, 0));
        assert!(queue.has_contiguity_between(5, 5));
    }

    #[test]
    fn test_has_contiguous_sequence_target_equals_expected() {
        let queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::default();

        // When target == expected_first, should return true immediately (edge case for eviction check)
        assert!(queue.has_contiguity_between(5, 5));
        assert!(queue.has_contiguity_between(0, 0));
        assert!(queue.has_contiguity_between(100, 100));
    }

    #[test]
    fn test_has_contiguous_sequence_wrong_start() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::default();

        queue.txs.insert(3, create_mock_queued_tx(3));
        queue.txs.insert(4, create_mock_queued_tx(4));
        queue.txs.insert(5, create_mock_queued_tx(5));

        // Queue starts at 3, but we expect 0
        assert!(!queue.has_contiguity_between(0, 6));
        // Queue starts at 3, but we expect 1
        assert!(!queue.has_contiguity_between(1, 6));
        // Queue starts at 3, and we expect 3 - should work
        assert!(queue.has_contiguity_between(3, 6));
    }

    #[test]
    fn test_has_contiguous_sequence_with_gap() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::default();

        queue.txs.insert(0, create_mock_queued_tx(0));
        queue.txs.insert(1, create_mock_queued_tx(1));
        queue.txs.insert(3, create_mock_queued_tx(3)); // Gap at 2
        queue.txs.insert(4, create_mock_queued_tx(4));

        // Should succeed up to the gap
        assert!(queue.has_contiguity_between(0, 0));
        assert!(queue.has_contiguity_between(0, 1));
        assert!(queue.has_contiguity_between(0, 2));

        // Should fail when target is beyond the gap
        assert!(!queue.has_contiguity_between(0, 3));
        assert!(!queue.has_contiguity_between(0, 4));
    }

    #[test]
    fn test_has_contiguous_sequence_complete() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::default();

        queue.txs.insert(5, create_mock_queued_tx(5));
        queue.txs.insert(6, create_mock_queued_tx(6));
        queue.txs.insert(7, create_mock_queued_tx(7));
        queue.txs.insert(8, create_mock_queued_tx(8));

        // Should succeed for all nonces we have, and the next valid one
        assert!(queue.has_contiguity_between(5, 5));
        assert!(queue.has_contiguity_between(5, 6));
        assert!(queue.has_contiguity_between(5, 7));
        assert!(queue.has_contiguity_between(5, 8));
        assert!(queue.has_contiguity_between(5, 9));

        // Should fail when target is beyond what we have
        assert!(!queue.has_contiguity_between(5, 10));
    }

    #[test]
    fn test_non_persisted() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::default();

        // Initially, no non-persisted nonces are known/tracked
        assert!(queue.non_persisted.user_nonce().is_none());
        assert!(queue
            .non_persisted
            .user_nonce_to_use_as_prerequisite_start()
            .is_none());

        // When a tx is in flight, the user's nonce is not known to be incremented yet
        queue.non_persisted.mark_inflight(0);
        assert!(queue.non_persisted.user_nonce().is_none());
        // But we take the in-flight tx into account to avoid evicting following ones
        assert_eq!(
            queue
                .non_persisted
                .user_nonce_to_use_as_prerequisite_start(),
            Some(1)
        );

        // If the in-flight tx failed we're back to square one
        queue.non_persisted.mark_inflight_execution_failed();
        assert!(queue.non_persisted.user_nonce().is_none());
        assert!(queue
            .non_persisted
            .user_nonce_to_use_as_prerequisite_start()
            .is_none());
    }

    #[test]
    fn test_determine_action() {
        // Tx nonce in the past
        assert_eq!(determine_action(3, 5, 10), Action::Reject);
        assert_eq!(determine_action(4, 5, 10), Action::Reject);
        assert_eq!(determine_action(0, 1, 0), Action::Reject);
        // Tx nonce equal to user nonce
        assert_eq!(determine_action(5, 5, 10), Action::Execute);
        assert_eq!(determine_action(5, 5, 0), Action::Execute);
        // Tx nonce within queue limit
        assert_eq!(determine_action(10, 5, 10), Action::Enqueue);
        assert_eq!(determine_action(15, 5, 10), Action::Enqueue);
        assert_eq!(determine_action(1, 0, 1), Action::Enqueue);
        // Tx nonce past queue limit
        assert_eq!(determine_action(16, 5, 10), Action::Reject);
        assert_eq!(determine_action(2, 0, 1), Action::Reject);
        assert_eq!(determine_action(1, 0, 0), Action::Reject);
    }

    /// Mock implementation of TxExecutionBackend for testing handle_new_tx
    #[derive(Clone)]
    struct MockTxExecutionBackend {
        api_nonce: Arc<AtomicU64>,
        executor_nonce: Arc<AtomicU64>,
        tx_queue_id: Arc<AtomicU64>,
        executed_txs: Arc<Mutex<Vec<(TxHash, u64)>>>,
        should_fail_nonce: Arc<Mutex<Option<u64>>>,
        downtime_after_nonce: Option<u64>,
        execution_delay: Duration,
        db_delay: Duration,
    }

    #[allow(dead_code)]
    impl MockTxExecutionBackend {
        fn new() -> Self {
            Self {
                api_nonce: Arc::new(AtomicU64::new(0)),
                executor_nonce: Arc::new(AtomicU64::new(0)),
                tx_queue_id: Arc::new(AtomicU64::new(0)),
                executed_txs: Arc::new(Mutex::new(Vec::new())),
                should_fail_nonce: Arc::new(Mutex::new(None)),
                downtime_after_nonce: None,
                execution_delay: Duration::from_millis(20),
                db_delay: Duration::from_millis(0),
            }
        }

        fn with_current_nonce(self, nonce: u64) -> Self {
            self.api_nonce.store(nonce, Ordering::SeqCst);
            self.executor_nonce.store(nonce, Ordering::SeqCst);
            self
        }

        fn with_tx_queue_id(self, tx_queue_id: u64) -> Self {
            self.tx_queue_id.store(tx_queue_id, Ordering::SeqCst);
            self
        }

        fn with_execution_delay(mut self, delay: Duration) -> Self {
            self.execution_delay = delay;
            self
        }

        fn with_db_delay(mut self, delay: Duration) -> Self {
            self.db_delay = delay;
            self
        }

        fn with_failure_at_nonce(self, nonce: u64) -> Self {
            *self.should_fail_nonce.lock().unwrap() = Some(nonce);
            self
        }

        fn with_downtime_after_nonce(mut self, nonce: u64) -> Self {
            self.downtime_after_nonce = Some(nonce);
            self
        }

        fn get_executed_nonces(&self) -> Vec<u64> {
            self.executed_txs
                .lock()
                .unwrap()
                .iter()
                .map(|(_, n)| *n)
                .collect()
        }
    }

    #[async_trait]
    impl TxExecutionBackend<TestSpec, TestRuntime> for MockTxExecutionBackend {
        fn get_current_nonce_for_user(&self, _credential_id: &CredentialId) -> u64 {
            self.api_nonce.load(Ordering::SeqCst)
        }

        fn get_current_executor_tx_queue_id(&self) -> u64 {
            self.tx_queue_id.load(Ordering::SeqCst)
        }

        async fn execute_tx(
            &self,
            baked_tx: &FullyBakedTx,
            tx_hash: TxHash,
            _ip_addr_and_credential: IpAndCredentialId<<TestSpec as Spec>::Address>,
            _original_tx_queue_id: u64,
            _reason: &'static str,
        ) -> TransactionReceiverResult<TestSpec, TestRuntime> {
            // Simulate execution delay (state transition time)
            if !self.execution_delay.is_zero() {
                tokio::time::sleep(self.execution_delay).await;
            }

            // Extract nonce from TX data (our mock TXs use first byte as nonce)
            let tx_nonce = *baked_tx.data.first().unwrap() as u64;

            // Check if we should fail this nonce
            let mut should_fail_nonce = self.should_fail_nonce.lock().unwrap();
            if *should_fail_nonce == Some(tx_nonce) {
                *should_fail_nonce = None;
                let receipt = TransactionReceipt {
                    tx_hash,
                    body_to_save: None,
                    events: Vec::new(),
                    receipt: sov_rollup_interface::stf::TxEffect::Skipped(SkippedTxContents {
                        gas_used: <TestSpec as Spec>::Gas::from([0, 0]),
                        error: TxProcessingError::RejectedByPreFlight,
                    }),
                };
                return Ok(Err(AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                    RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
                ))));
            }

            if let Some(downtime_after_nonce) = self.downtime_after_nonce {
                if tx_nonce > downtime_after_nonce {
                    return Ok(Err(AcceptTxError::NotFullySynced(
                        SequencerNotReadyDetails::Syncing {
                            target_da_height: 100,
                            synced_da_height: 50,
                        },
                    )));
                }
            }

            let executor_nonce = self.executor_nonce.load(Ordering::SeqCst);
            if executor_nonce != tx_nonce {
                let receipt = TransactionReceipt {
                    tx_hash,
                    body_to_save: None,
                    events: Vec::new(),
                    receipt: sov_rollup_interface::stf::TxEffect::Skipped(SkippedTxContents {
                        gas_used: <TestSpec as Spec>::Gas::from([0, 0]),
                        error: TxProcessingError::CheckUniquenessFailed(format!("Tx bad nonce for credential id: {}, expected: {executor_nonce}, but found: {tx_nonce}", CredentialId::from_bytes([1u8; 32]))),
                    }),
                };
                return Ok(Err(AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                    RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
                ))));
            }

            // Track execution
            self.executed_txs.lock().unwrap().push((tx_hash, tx_nonce));
            self.executor_nonce.store(tx_nonce + 1, Ordering::SeqCst);

            // Create oneshot channel for DB persistence simulation
            let (tx, rx) = oneshot::channel();

            // Spawn task to simulate DB persistence delay
            // The nonce is only incremented after the DB operation "completes"
            let backend = self.clone();
            let baked_tx_clone = baked_tx.clone();
            tokio::spawn(async move {
                // Simulate DB persistence delay
                if !backend.db_delay.is_zero() {
                    tokio::time::sleep(backend.db_delay).await;
                }

                // Increment nonce (simulates API state update after DB persistence)
                backend.api_nonce.store(tx_nonce + 1, Ordering::SeqCst);

                // Send result
                let _ = tx.send(AcceptedTx::<Confirmation<TestSpec, TestRuntime>> {
                    tx: baked_tx_clone,
                    tx_hash,
                    confirmation: Confirmation {
                        events: vec![],
                        receipt: sov_modules_api::ApiTxEffect::Skipped {
                            data: SkippedTxContents {
                                gas_used: <TestSpec as Spec>::Gas::from([0, 0]),
                                error: TxProcessingError::RejectedByPreFlight,
                            },
                        },
                        tx_number: 0,
                    },
                });
            });

            Ok(Ok(rx))
        }
    }

    /// Helper to create a default task for tests that don't care about specific config
    /// Returns (sender, backend, _shutdown_sender) - the shutdown sender must be kept alive
    /// for the duration of the test or the task will exit immediately.
    fn test_buffer_task(
        backend: &MockTxExecutionBackend,
        timeout_override: Option<u64>,
    ) -> (
        NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        watch::Sender<()>,
    ) {
        let (shutdown_sender, shutdown_receiver) = watch::channel(());
        let (_forced_tx_batch_notifier, forced_tx_batch_receiver) = broadcast::channel(1);
        let (_handle, sender) = NonceBufferTask::spawn(
            backend.clone(),
            DEFAULT_TEST_MAX_QUEUE_SIZE,
            timeout_override.unwrap_or(DEFAULT_TEST_QUEUE_TIMEOUT_MS),
            forced_tx_batch_receiver,
            shutdown_receiver,
        );
        (sender, shutdown_sender)
    }

    fn default_test_buffer_task() -> (
        MockTxExecutionBackend,
        NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        watch::Sender<()>,
    ) {
        let backend = MockTxExecutionBackend::new();
        let (sender, shutdown_sender) = test_buffer_task(&backend, None);
        (backend, sender, shutdown_sender)
    }

    type DrainTaskParts = (
        NonceBufferTask<MockTxExecutionBackend, TestSpec, TestRuntime>,
        mpsc::Sender<NonceBufferInput<TestSpec, TestRuntime>>,
        watch::Sender<()>,
        watch::Receiver<()>,
    );

    fn build_task_with_channels(backend: MockTxExecutionBackend) -> DrainTaskParts {
        let (buffer_sender_channel, buffer_input) = mpsc::channel(MAX_BUFFER_INPUT_QUEUE);
        let (timeout_sender, _timeout_receiver) = mpsc::channel(MAX_BUFFERED_TXS);
        let (shutdown_sender, shutdown_receiver) = watch::channel(());
        let (_forced_tx_batch_notifier, forced_tx_batch_receiver) = broadcast::channel(1);
        let input_sender = NonceBufferInputSender {
            buffer_sender_channel: buffer_sender_channel.clone(),
            execution_backend: backend.clone(),
            shutdown_receiver: shutdown_sender.subscribe(),
        };
        let task = NonceBufferTask {
            buffers: Default::default(),
            buffer_input,
            forced_tx_batch_receiver,
            input_sender,
            execution_backend: backend,
            maximum_future_nonce_delta: DEFAULT_TEST_MAX_QUEUE_SIZE,
            timeout_sender,
            timeout_metrics_batcher: MetricBatcher::new(METRICS_BATCH_SIZE),
            main_queue_depth_batcher: MetricBatcher::new(METRICS_BATCH_SIZE),
        };
        (
            task,
            buffer_sender_channel,
            shutdown_sender,
            shutdown_receiver,
        )
    }

    async fn push_new_tx_input(
        input_sender: &mpsc::Sender<NonceBufferInput<TestSpec, TestRuntime>>,
        nonce: u8,
        hash: [u8; 32],
    ) -> oneshot::Receiver<TransactionReceiverResult<TestSpec, TestRuntime>> {
        let queued = create_mock_queued_tx_with_hash(nonce, hash);
        let (result_sender, result_receiver) = oneshot::channel();
        input_sender
            .send(NonceBufferInput::NewTx {
                baked_tx: queued.baked_tx,
                tx_hash: queued.tx_hash,
                ip_addr_and_credential: queued.ip_addr_and_credential,
                tx_nonce: nonce.into(),
                original_tx_queue_id: queued.original_tx_queue_id,
                result_sender,
            })
            .await
            .unwrap();
        result_receiver
    }

    async fn push_tx_executed_input(
        input_sender: &mpsc::Sender<NonceBufferInput<TestSpec, TestRuntime>>,
        credential_id: CredentialId,
        tx_nonce: u64,
        tx_result: TransactionReceiverResult<TestSpec, TestRuntime>,
    ) -> oneshot::Receiver<TransactionReceiverResult<TestSpec, TestRuntime>> {
        let (result_sender, result_receiver) = oneshot::channel();
        input_sender
            .send(NonceBufferInput::TxExecuted {
                credential_id,
                tx_nonce,
                tx_result,
                result_sender,
            })
            .await
            .unwrap();
        result_receiver
    }

    fn insert_timed_out_tx(
        task: &mut NonceBufferTask<MockTxExecutionBackend, TestSpec, TestRuntime>,
        nonce: u8,
        hash: [u8; 32],
    ) -> (
        oneshot::Receiver<TransactionReceiverResult<TestSpec, TestRuntime>>,
        CredentialId,
        TxHash,
    ) {
        let mut queued = create_mock_queued_tx_with_hash(nonce, hash);
        let (result_sender, result_receiver) = oneshot::channel();
        queued.result_sender = result_sender;
        let credential_id = queued.ip_addr_and_credential.credential_id;
        task.buffers
            .entry(credential_id)
            .or_default()
            .txs
            .insert(nonce.into(), queued);
        (result_receiver, credential_id, TxHash::from(hash))
    }

    /// Spawns a transaction submission as a concurrent task.
    /// Includes a short sleep to allow the task to enter the buffer, ensuring transactions are
    /// submitted to the buffer in the order submit_single_transaction is called.
    async fn submit_single_transaction(
        sender: NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        nonce: u8,
    ) -> JoinHandle<TransactionReceiverResult<TestSpec, TestRuntime>> {
        submit_single_transaction_with_hash(sender, nonce, [nonce; 32]).await
    }

    /// Same as submit_single_transaction but allows overriding the hash, for submitting
    /// transactions with identical nonces but different hashes.
    async fn submit_single_transaction_with_hash(
        sender: NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        nonce: u8,
        hash: [u8; 32],
    ) -> JoinHandle<TransactionReceiverResult<TestSpec, TestRuntime>> {
        let to_queue = create_mock_queued_tx_with_hash(nonce, hash);
        let handle = tokio::spawn(async move {
            sender
                .handle_new_tx(
                    to_queue.baked_tx,
                    to_queue.tx_hash,
                    nonce.into(),
                    IpAndCredentialId {
                        address: <TestSpec as Spec>::Address::from([1; 28]),
                        credential_id: CredentialId::from([1u8; 32]),
                        ip_addr: std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
                    },
                    to_queue.original_tx_queue_id,
                )
                .await
        });
        // Give the spawned task time to send to the buffer
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle
    }

    async fn submit_transactions(
        sender: NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        nonces: Vec<u8>,
    ) -> Vec<JoinHandle<TransactionReceiverResult<TestSpec, TestRuntime>>> {
        let mut handles = Vec::with_capacity(nonces.len());
        for n in nonces {
            handles.push(submit_single_transaction(sender.clone(), n).await);
        }
        handles
    }

    fn get_test_nonce(backend: &MockTxExecutionBackend) -> u64 {
        backend.get_current_nonce_for_user(&CredentialId::from_bytes([1u8; 32]))
    }

    #[derive(Clone, Debug)]
    enum Outcome {
        Ok,
        Err(InvalidNonceReason),
        Revert,
        Invalidate503,
        NotFullySynced,
        Shutdown,
        StfNonceReject(u8, u8),
    }

    async fn assert_on_results(
        results: Vec<TransactionReceiverResult<TestSpec, TestRuntime>>,
        expected_outcomes: Vec<Outcome>,
    ) {
        let mut results = results.into_iter();
        let expected_outcomes = expected_outcomes.into_iter();

        for (i, outcome) in expected_outcomes.enumerate() {
            let result = results
                .next()
                .expect("Test passed more expected outcomes than there were tx results");
            match outcome {
                Outcome::Ok => assert!(
                    result.unwrap().unwrap().await.is_ok(),
                    "Expected tx {i} to succeed"
                ),
                Outcome::Err(reason) => {
                    let inner = result.expect("Expected Ok from TransactionReceiverResult");
                    let err =
                        inner.expect_err(&format!("Expected tx {i} to fail with nonce error"));

                    // Pattern match to extract the error message
                    match err {
                        AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                            RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
                        )) => {
                            match receipt.receipt {
                                sov_rollup_interface::stf::TxEffect::Skipped(contents) => {
                                    match contents.error {
                                        TxProcessingError::CheckUniquenessFailed(msg) => {
                                            // Verify the message contains "bad nonce"
                                            assert!(
                                                msg.contains("bad nonce"),
                                                "Tx {i}: Error should mention bad nonce: \"{msg}\"",
                                            );

                                            // Verify the reason-specific substring
                                            let expected_substring = match reason {
                                                InvalidNonceReason::Invalid => "did not attempt to queue",
                                                InvalidNonceReason::Timeout => "has timed out and is being evicted",
                                                InvalidNonceReason::EvictedBeforeExecution => "dropped from the queue for an unknown reason",

                                                InvalidNonceReason::Replaced => "new transaction with the same nonce has arrived and replaced it",
                                                InvalidNonceReason::AlreadyQueued => "identical transaction with the same hash",
                                            };
                                            assert!(
                                                msg.contains(expected_substring),
                                                "Tx {i}: Error message should contain \"{expected_substring}\" for reason {reason:?}, but got: \"{msg}\"",
                                            );
                                        }
                                        other => panic!("Tx {i}: Expected CheckUniquenessFailed error, got: {other:?}"),
                                    }
                                }
                                other => panic!("Tx {i}: Expected Skipped receipt, got: {other:?}"),
                            }
                        }
                        other => {
                            panic!("Tx {i}: Expected UnsuccessfulTransaction error, got: {other:?}")
                        }
                    }
                }
                Outcome::Revert => {
                    let inner = result.expect("Expected Ok from TransactionReceiverResult");
                    let err = inner.expect_err(&format!("Expected tx {i} to fail with rejection"));

                    assert!(
                        matches!(
                            err,
                            AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                                RollupBlockExecutorError::UnsuccessfulTransaction {
                                    receipt: TransactionReceipt {
                                        receipt: sov_rollup_interface::stf::TxEffect::Skipped(
                                            SkippedTxContents {
                                                error: TxProcessingError::RejectedByPreFlight,
                                                ..
                                            }
                                        ),
                                        ..
                                    }
                                },
                            ))
                        ),
                        "Tx {i}: Expected RejectedByPreFlight rejection, got: {err:?}"
                    );
                }
                Outcome::Invalidate503 => {
                    let inner = result.expect("Expected Ok from TransactionReceiverResult");
                    let err = inner.expect_err(&format!("Expected tx {i} to fail with rejection"));

                    assert!(
                        matches!(err, AcceptTxError::SequencerOverloaded503),
                        "Tx {i}: Expected SequencerOverloaded503 rejection, got: {err:?}"
                    );
                }
                Outcome::NotFullySynced => {
                    let inner = result.expect("Expected Ok from TransactionReceiverResult");
                    let err =
                        inner.expect_err(&format!("Expected tx {i} to fail with NotFullySynced"));

                    assert!(
                        matches!(err, AcceptTxError::NotFullySynced(_)),
                        "Tx {i}: Expected NotFullySynced rejection, got: {err:?}"
                    );
                }
                Outcome::Shutdown => {
                    assert!(
                        matches!(result, Err(SequencerStateUpdatorError::Shutdown)),
                        "Tx {i}: Expected shutdown error, got: {result:?}"
                    );
                }
                Outcome::StfNonceReject(expected, tx) => {
                    let inner = result.expect("Expected Ok from TransactionReceiverResult");
                    let err = inner.expect_err(&format!("Expected tx {i} to fail with rejection"));

                    let expected_msg = format!(
                        "Tx bad nonce for credential id: {}, expected: {expected}, but found: {tx}",
                        CredentialId::from_bytes([1u8; 32])
                    );
                    assert!(
                        matches!(
                            &err,
                            AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                                RollupBlockExecutorError::UnsuccessfulTransaction {
                                    receipt: TransactionReceipt {
                                        receipt: sov_rollup_interface::stf::TxEffect::Skipped(SkippedTxContents {
                                            error: TxProcessingError::CheckUniquenessFailed(msg),
                                            ..
                                        }),
                                        ..
                                    }
                                },
                            )) if msg == &expected_msg
                        ),
                        "Tx {i}: Expected STF nonce rejection with expected={expected}, tx={tx}, got: {err:?}"
                    );
                }
            }
        }
        assert!(
            results.next().is_none(),
            "Test passed more tx results than there were expected outcomes"
        );
    }

    /// Helper to collect results from JoinHandles
    async fn collect_results(
        handles: Vec<JoinHandle<TransactionReceiverResult<TestSpec, TestRuntime>>>,
    ) -> Vec<TransactionReceiverResult<TestSpec, TestRuntime>> {
        futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.expect("JoinHandle panicked"))
            .collect()
    }

    #[tokio::test]
    async fn test_rejects_stale_nonces() {
        let (backend, sender, _shutdown) = default_test_buffer_task();
        let backend = backend.with_current_nonce(5);

        let handles = submit_transactions(sender, vec![3, 4, 5]).await;
        let results = collect_results(handles).await;

        assert_eq!(backend.get_executed_nonces(), vec![5]);
        assert_on_results(
            results,
            vec![
                Outcome::Err(InvalidNonceReason::Invalid),
                Outcome::Err(InvalidNonceReason::Invalid),
                Outcome::Ok,
            ],
        )
        .await;
        assert_eq!(get_test_nonce(&backend), 6);
    }

    #[tokio::test]
    async fn test_rejects_too_far_future_nonce() {
        let (backend, sender, _shutdown) = default_test_buffer_task();

        let handles = submit_transactions(
            sender,
            vec![
                DEFAULT_TEST_MAX_QUEUE_SIZE as u8 + 1,
                DEFAULT_TEST_MAX_QUEUE_SIZE as u8,
            ],
        )
        .await;
        let results = collect_results(handles).await;

        assert!(backend.get_executed_nonces().is_empty());
        // First tx should have been rejected as invalid. Second one should be right at the limit
        // and so should have been queued (and timed out)
        assert_on_results(
            results,
            vec![
                Outcome::Err(InvalidNonceReason::Invalid),
                Outcome::Err(InvalidNonceReason::Timeout),
            ],
        )
        .await;
        assert_eq!(get_test_nonce(&backend), 0);
    }

    #[tokio::test]
    async fn test_execution_stops_at_gap_and_times_out() {
        let (backend, sender, _shutdown) = default_test_buffer_task();

        let handles = submit_transactions(sender, vec![0, 1, 3, 4]).await;
        let results = collect_results(handles).await;

        // Should have executed exactly 2 transactions (0 and 1), then stopped at gap
        assert_on_results(
            results,
            vec![
                Outcome::Ok,
                Outcome::Ok,
                Outcome::Err(InvalidNonceReason::Timeout),
                Outcome::Err(InvalidNonceReason::Timeout),
            ],
        )
        .await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_execution_triggers_drain_of_queued_txs() {
        let backend = MockTxExecutionBackend::new();
        let (sender, _shutdown) = test_buffer_task(&backend, Some(500));

        let mut handles = submit_transactions(sender.clone(), vec![1, 2, 3]).await;
        assert!(backend.get_executed_nonces().is_empty());
        assert_eq!(get_test_nonce(&backend), 0);

        // Submit TX with nonce 0 (current nonce) - should execute immediately and trigger drain
        let handle_0 = submit_single_transaction(sender, 0).await;
        handles.insert(0, handle_0);

        let results = collect_results(handles).await;
        assert_on_results(results, vec![Outcome::Ok; 4]).await;

        assert_eq!(backend.get_executed_nonces(), vec![0, 1, 2, 3]);
        assert_eq!(get_test_nonce(&backend), 4);
    }

    /// Ignored because pre-requisite checks are disabled, so transactions will timeout
    /// immediately.
    #[tokio::test]
    #[ignore]
    async fn test_queued_tx_timeout_loops_if_all_prerequisites_present() {
        // Timeout longer than execution delay but shorter than total time to execute all
        // transactions, to ensure we hit the timeout loop
        let backend =
            MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(200));
        let (sender, _shutdown) = test_buffer_task(&backend, Some(500));

        // Submit TXs with nonces 1-10 (all will queue)
        let mut handles = submit_transactions(sender.clone(), (1..11).collect()).await;

        // Now submit TX with nonce 0 to trigger drain
        let handle_0 = submit_single_transaction(sender.clone(), 0).await;
        handles.insert(0, handle_0);

        // All transactions should succeed, despite timeouts being scheduled
        let results = collect_results(handles).await;
        assert_on_results(results, vec![Outcome::Ok; 11]).await;

        assert_eq!(backend.get_executed_nonces(), (0..11).collect::<Vec<_>>());
        assert_eq!(get_test_nonce(&backend), 11);
    }

    #[tokio::test]
    async fn test_non_persisted_nonce_tracking_slow_db() {
        // This test verifies that the queue correctly tracks nonces before they're persisted to
        // the DB.
        // - Tx 0 executes successfully, but DB persistence is slow
        // - Tx 1 arrives before DB write completes: so API state still has old nonce (0)
        // If the queue used API state: tx 1 would see current_nonce=0 and get queued, eventually
        // timing out. But to be correct, the queue needs to execute tx 1 immediately, since tx 0
        // already succeeded.
        let backend = MockTxExecutionBackend::new().with_db_delay(Duration::from_millis(500)); // Slow DB
        let (sender, _shutdown) = test_buffer_task(&backend, Some(200)); // Queue times out faster than DB

        let result_0 = submit_single_transaction(sender.clone(), 0)
            .await
            .await
            .expect("JoinHandle panicked");
        // At this point tx 0 has been sent to the buffer and is executing
        // We submit tx 1 immediately and verify it isn't rejected and doesn't timeout
        let result_1 = submit_single_transaction(sender, 1)
            .await
            .await
            .expect("JoinHandle panicked");
        assert_on_results(vec![result_0, result_1], vec![Outcome::Ok, Outcome::Ok]).await;

        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    /// Ignored because pre-requisite checks are disabled, so tx 1 will time out immediately.
    #[tokio::test]
    #[ignore]
    async fn test_non_persisted_nonce_tracking_slow_execution() {
        // This test verifies that the queue avoids evicting transactions if a current one is
        // executing. Similar to the db timeout test.
        // - Tx 0 executes slowly
        // - Tx 1 arrives mid-execution. The queue should A) not queue it for execution yet - maybe
        // tx 0 will fail. But also B) not evict it - if tx 0 succeeds then tx 1 can be executed.
        let backend =
            MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(500)); // Slow execution
        let (sender, _shutdown) = test_buffer_task(&backend, Some(200)); // Queue times out faster than execution

        let handle_0 = submit_single_transaction(sender.clone(), 0).await;
        // At this point transaction 0 is executing slowly
        // We submit tx 1 immediately and verify it isn't rejected and doesn't timeout
        let handle_1 = submit_single_transaction(sender, 1).await;
        // Both transactions should succeed
        let results = collect_results(vec![handle_0, handle_1]).await;
        assert_on_results(results, vec![Outcome::Ok, Outcome::Ok]).await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    /// Ignored because pre-requisite checks are disabled, so tx 1 will time out immediately.
    #[tokio::test]
    #[ignore]
    async fn test_non_persisted_nonce_tracking_slow_execution_with_rejection() {
        // This test verifies correct behaviour if a transaction arrives while the previous one is
        // in-flight, and then the previous one rejects.
        // - Tx 0 executes slowly
        // - Tx 1 arrives mid-execution. The queue should not queue it for execution yet - maybe
        // tx 0 will fail. Once tx 0 fails, then tx 1 will time out.
        let backend = MockTxExecutionBackend::new()
            .with_execution_delay(Duration::from_millis(500))
            .with_failure_at_nonce(0);
        let (sender, _shutdown) = test_buffer_task(&backend, Some(200)); // Queue times out faster than execution

        let handle_0 = submit_single_transaction(sender.clone(), 0).await;
        // At this point transaction 0 is executing slowly
        // We submit tx 1 immediately and verify it isn't rejected (as it would be if the queue
        // submitted it to the STF), but rather times out
        let handle_1 = submit_single_transaction(sender, 1).await;
        let results = collect_results(vec![handle_0, handle_1]).await;
        assert_on_results(
            results,
            vec![Outcome::Revert, Outcome::Err(InvalidNonceReason::Timeout)],
        )
        .await;
        assert!(backend.get_executed_nonces().is_empty());
        assert_eq!(get_test_nonce(&backend), 0);
    }

    #[tokio::test]
    async fn test_replacement_of_queued_transaction() {
        let backend =
            MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(200));
        let (sender, _shutdown) = test_buffer_task(&backend, Some(1000)); // Long timeout to avoid timeouts

        let handle_0 = submit_single_transaction(sender.clone(), 0).await;
        // Tx 0 is now executing slowly

        // Submit tx 1a (will be queued)
        let handle_1a = submit_single_transaction(sender.clone(), 1).await;

        // Submit tx 1b with same nonce but different hash (should replace tx 1a)
        let handle_1b = submit_single_transaction_with_hash(sender.clone(), 1, [200; 32]).await;

        // Wait for all to complete
        let results = collect_results(vec![handle_0, handle_1a, handle_1b]).await;

        assert_on_results(
            results,
            vec![
                Outcome::Ok,
                Outcome::Err(InvalidNonceReason::Replaced),
                Outcome::Ok,
            ],
        )
        .await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_replacement_with_same_hash() {
        let backend =
            MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(200));
        let (sender, _shutdown) = test_buffer_task(&backend, Some(1000)); // Long timeout to avoid timeouts

        let handle_0 = submit_single_transaction(sender.clone(), 0).await;
        // Tx 0 is now executing slowly

        // Submit tx 1a (will be queued)
        let handle_1a = submit_single_transaction(sender.clone(), 1).await;

        // Submit tx 1b with same nonce and same hash
        let handle_1b = submit_single_transaction(sender.clone(), 1).await;

        // Wait for all to complete
        let results = collect_results(vec![handle_0, handle_1a, handle_1b]).await;

        assert_on_results(
            results,
            vec![
                Outcome::Ok,
                Outcome::Ok,
                Outcome::Err(InvalidNonceReason::AlreadyQueued),
            ],
        )
        .await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_replacement_of_inflight_transaction_success() {
        let backend =
            MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(300));
        let (sender, _shutdown) = test_buffer_task(&backend, Some(1000)); // Long timeout

        // Submit tx 0a which will execute slowly
        let handle_0a = submit_single_transaction(sender.clone(), 0).await;

        // Submit tx 0b with same nonce (should be queued since 0a is in-flight)
        let handle_0b = submit_single_transaction_with_hash(sender.clone(), 0, [200; 32]).await;

        // Submit tx 1 (should be queued)
        let handle_1 = submit_single_transaction(sender.clone(), 1).await;

        // Wait for all to complete
        let results = collect_results(vec![handle_0a, handle_0b, handle_1]).await;

        // Tx 0a succeeds, tx 0b is rejected as Invalid (stale nonce), tx 1 succeeds
        assert_on_results(
            results,
            vec![Outcome::Ok, Outcome::StfNonceReject(1, 0), Outcome::Ok],
        )
        .await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_replacement_of_inflight_transaction_failure() {
        let backend = MockTxExecutionBackend::new()
            .with_execution_delay(Duration::from_millis(300))
            .with_failure_at_nonce(0);
        let (sender, _shutdown) = test_buffer_task(&backend, Some(1000)); // Long timeout

        // Submit tx 0a which will execute slowly and fail
        let handle_0a = submit_single_transaction(sender.clone(), 0).await;

        // Submit tx 0b with same nonce (should be queued since 0a is in-flight)
        let handle_0b = submit_single_transaction_with_hash(sender.clone(), 0, [200; 32]).await;

        // Submit tx 1 (should be queued)
        let handle_1 = submit_single_transaction(sender.clone(), 1).await;

        // Wait for all to complete
        let results = collect_results(vec![handle_0a, handle_0b, handle_1]).await;

        // Tx 0a is rejected by backend, tx 0b succeeds (executed after 0a failed), tx 1 succeeds
        assert_on_results(results, vec![Outcome::Revert, Outcome::Ok, Outcome::Ok]).await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_queue_id_increment_invalidates_buffer() {
        let backend =
            MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(300));
        let (sender, _shutdown) = test_buffer_task(&backend, Some(3000)); // Long timeout

        // Submit txs 0 and 1
        let handles_a = submit_transactions(sender.clone(), (0..2).collect()).await;
        // Submit txs 2, 3, 4
        let handles_b = submit_transactions(sender.clone(), (2..5).collect()).await;

        // Now we await exactly 0 and 1
        let results_a = collect_results(handles_a).await;
        // At this point, transaction 2 should be mid-execution, since it takes a while to execute.
        // But txs 3 and 4 will still be in the queue and should be invalidated now.
        let backend = backend.with_tx_queue_id(1);
        let results_b = collect_results(handles_b).await;

        let mut results = results_a;
        results.extend(results_b);
        assert_on_results(
            results,
            vec![
                Outcome::Ok,
                Outcome::Ok,
                Outcome::Ok,
                Outcome::Invalidate503,
                Outcome::Invalidate503,
            ],
        )
        .await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1, 2]);
        assert_eq!(get_test_nonce(&backend), 3);
    }

    #[tokio::test]
    async fn test_sequencer_downtime_invalidates_buffer() {
        let backend = MockTxExecutionBackend::new().with_downtime_after_nonce(1);
        let (sender, _shutdown) = test_buffer_task(&backend, Some(2000)); // Long timeout

        // Txs 0 and 1 will succeed, 2 will hit the resync
        let handles = submit_transactions(sender.clone(), (0..5).collect()).await;

        // Now we await exactly 0 and 1
        let results = collect_results(handles).await;
        assert_on_results(
            results,
            vec![
                Outcome::Ok,
                Outcome::Ok,
                Outcome::NotFullySynced,
                Outcome::Invalidate503,
                Outcome::Invalidate503,
            ],
        )
        .await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_shutdown_invalidates_buffer() {
        let backend =
            MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(200));
        let (sender, shutdown_sender) = test_buffer_task(&backend, Some(2000)); // Long timeout

        let handles = submit_transactions(sender.clone(), (0..5).collect()).await;
        tokio::time::sleep(Duration::from_millis(450)).await;
        let _ = shutdown_sender.send(());
        let results = collect_results(handles).await;
        let executed = backend.get_executed_nonces().len();
        assert!(
            executed > 0 && executed < 5,
            "Shutdown should interrupt queued transactions; executed={executed}"
        );
        let mut expected = Vec::with_capacity(5);
        expected.extend(std::iter::repeat_n(Outcome::Ok, executed));
        expected.extend(std::iter::repeat_n(Outcome::Shutdown, 5 - executed));
        assert_on_results(results, expected).await;
        let expected_nonces: Vec<u64> = (0..executed as u64).collect();
        assert_eq!(backend.get_executed_nonces(), expected_nonces);
    }

    #[tokio::test]
    async fn test_shutdown_drains_pending_messages() {
        let backend = MockTxExecutionBackend::new();
        let (mut task, input_sender, shutdown_sender, mut shutdown_receiver) =
            build_task_with_channels(backend);

        let new_tx_receiver = push_new_tx_input(&input_sender, 1, [1; 32]).await;
        let (timed_out_receiver, credential_id, timed_out_hash) =
            insert_timed_out_tx(&mut task, 2, [9; 32]);
        input_sender
            .send(NonceBufferInput::TxTimedOut {
                credential_id,
                tx_nonce: 2,
                tx_hash: timed_out_hash,
            })
            .await
            .unwrap();
        let tx_result = Ok(Err(AcceptTxError::NotFullySynced(
            SequencerNotReadyDetails::Syncing {
                target_da_height: 100,
                synced_da_height: 50,
            },
        )));
        let executed_receiver =
            push_tx_executed_input(&input_sender, CredentialId::from([1u8; 32]), 7, tx_result)
                .await;
        input_sender
            .send(NonceBufferInput::TxPersisted {
                credential_id: CredentialId::from([1u8; 32]),
                tx_nonce: 7,
            })
            .await
            .unwrap();
        drop(input_sender);
        let _ = shutdown_sender.send(());

        let run_handle = tokio::spawn(async move {
            task.run(&mut shutdown_receiver).await;
        });
        run_handle.await.unwrap();

        let new_tx_result = new_tx_receiver.await.unwrap();
        let timed_out_result = timed_out_receiver.await.unwrap();
        let executed_result = executed_receiver.await.unwrap();
        assert_on_results(
            vec![new_tx_result, timed_out_result, executed_result],
            vec![
                Outcome::Shutdown,
                Outcome::Err(InvalidNonceReason::Timeout),
                Outcome::NotFullySynced,
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn test_wipe_drains_pending_messages() {
        let backend = MockTxExecutionBackend::new().with_tx_queue_id(1);
        let (mut task, input_sender, shutdown_sender, mut shutdown_receiver) =
            build_task_with_channels(backend);

        let trigger_new_tx_receiver = push_new_tx_input(&input_sender, 0, [1; 32]).await;
        let queued_new_tx_receiver = push_new_tx_input(&input_sender, 1, [2; 32]).await;
        let (timed_out_receiver, credential_id, timed_out_hash) =
            insert_timed_out_tx(&mut task, 2, [7; 32]);
        input_sender
            .send(NonceBufferInput::TxTimedOut {
                credential_id,
                tx_nonce: 2,
                tx_hash: timed_out_hash,
            })
            .await
            .unwrap();
        let tx_result = Ok(Err(AcceptTxError::NotFullySynced(
            SequencerNotReadyDetails::Syncing {
                target_da_height: 100,
                synced_da_height: 50,
            },
        )));
        let executed_receiver =
            push_tx_executed_input(&input_sender, CredentialId::from([1u8; 32]), 9, tx_result)
                .await;
        input_sender
            .send(NonceBufferInput::TxPersisted {
                credential_id: CredentialId::from([1u8; 32]),
                tx_nonce: 9,
            })
            .await
            .unwrap();

        let run_handle = tokio::spawn(async move {
            task.run(&mut shutdown_receiver).await;
        });

        let trigger_result = tokio::time::timeout(Duration::from_secs(2), trigger_new_tx_receiver)
            .await
            .expect("Timed out waiting for trigger_result")
            .unwrap();
        let queued_result = tokio::time::timeout(Duration::from_secs(2), queued_new_tx_receiver)
            .await
            .expect("Timed out waiting for queued_result")
            .unwrap();
        let timed_out_result = tokio::time::timeout(Duration::from_secs(2), timed_out_receiver)
            .await
            .expect("Timed out waiting for timed_out_result")
            .unwrap();
        let executed_result = tokio::time::timeout(Duration::from_secs(2), executed_receiver)
            .await
            .expect("Timed out waiting for executed_result")
            .unwrap();

        assert_on_results(
            vec![
                trigger_result,
                queued_result,
                timed_out_result,
                executed_result,
            ],
            vec![
                Outcome::Invalidate503,
                Outcome::Invalidate503,
                Outcome::Err(InvalidNonceReason::Timeout),
                Outcome::NotFullySynced,
            ],
        )
        .await;

        let _ = shutdown_sender.send(());
        run_handle.await.unwrap();
    }
}
