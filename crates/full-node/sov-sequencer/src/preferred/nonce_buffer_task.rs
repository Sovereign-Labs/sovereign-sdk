#![allow(unused_imports)]
#![allow(dead_code)]
use async_trait::async_trait;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::rest::ApiState;
use sov_modules_api::{
    FullyBakedTx, Gas, Runtime, SkippedTxContents, Spec, TransactionReceipt, TxProcessingError,
};
use sov_rollup_interface::{crypto::CredentialId, TxHash};
use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::HashMap;
use std::collections::{btree_map::OccupiedEntry, BTreeMap};
use std::fmt::Debug;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::common::AcceptedTx;

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

pub(crate) type TransactionExecutorResult<S, Rt> =
    Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>;

pub(crate) type TransactionReceiverResult<S, Rt> =
    Result<TransactionExecutorResult<S, Rt>, SequencerStateUpdatorError>;

struct QueuedTx<S: Spec, Rt: Runtime<S>> {
    pub baked_tx: FullyBakedTx,
    pub tx_hash: TxHash,
    pub original_tx_queue_id: u64,
    pub nonce_when_queued: u64,
    pub queued_at: Instant,
    pub result_sender: oneshot::Sender<TransactionReceiverResult<S, Rt>>,
}

/// Tracks the nonce for transactions which have been executed through the queue, but haven't been
/// persisted to database (and therefore to state) yet.
#[derive(Default)]
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
        self.user_nonce().map(|n| match self.has_in_flight {
            true => n.checked_add(1).expect("Overflow adding 1 to user nonce"),
            false => n,
        })
    }

    fn mark_inflight(&mut self, tx_nonce_check: u64) {
        if self
            .user_nonce()
            .is_some_and(|user_nonce| tx_nonce_check != user_nonce)
        {
            tracing::error!("Sequencer nonce buffer: non-persisted tracking: attempted to execute nonce that does not match tracked next nonce!");
            panic!("Sequencer nonce buffer: non-persisted tracking: attempted to execute nonce that does not match tracked next nonce!");
        }
        // We don't throw an error if has_in_flight is already true - for instance, if two txs with
        // the current valid nonce arrive near-simultaneously, we let the executor sort them out for
        // simplicity. Thus there can be more than one tx in flight - they just all need to have
        // the same nonce.
        self.has_in_flight = true;
    }

    fn mark_inflight_execution_succeeded(&mut self, tx_nonce_check: u64) {
        if !self.has_in_flight {
            tracing::error!("Sequencer nonce buffer: non-persisted tracking: attempted to mark executed tx successful when has_in_flight is false!");
            panic!("Sequencer nonce buffer: non-persisted tracking: attempted to mark executed tx successful when has_in_flight is false!");
        }
        self.has_in_flight = false;

        self.last_successfully_executed = self.last_successfully_executed.map(|last| {
            let new = last.checked_add(1).expect("Overflow adding 1 to user nonce");
            if new != tx_nonce_check {
                tracing::error!("Sequencer nonce buffer: non-persisted tracking: after executing, incremented nonce did not match tx nonce!");
                panic!("Sequencer nonce buffer: non-persisted tracking: after executing, incremented nonce did not match tx nonce!");
            }
            new
        }).or(Some(tx_nonce_check));
    }

    fn mark_inflight_execution_failed(&mut self, tx_nonce_check: u64) {
        if !self.has_in_flight {
            tracing::error!("Sequencer nonce buffer: non-persisted tracking: attempted to mark executed tx successful when has_in_flight is false!");
            panic!("Sequencer nonce buffer: non-persisted tracking: attempted to mark executed tx successful when has_in_flight is false!");
        }
        self.has_in_flight = false;

        if self.last_successfully_executed.is_some_and(|l| {
            l.checked_add(1).expect("Overflow adding 1 to user nonce") != tx_nonce_check
        }) {
            tracing::error!("Sequencer nonce buffer: non-persisted tracking: marked failed to execute, tx whose nonce was non-consecutive with last known one");
            panic!("Sequencer nonce buffer: non-persisted tracking: marked failed to execute, tx whose nonce was non-consecutive with last known one");
        }
    }
}

#[derive(Default)]
struct AddressQueue<S: Spec, Rt: Runtime<S>> {
    txs: BTreeMap<u64, QueuedTx<S, Rt>>,
    non_persisted: NonPersistedTxs,
}

impl<S: Spec, Rt: Runtime<S>> AddressQueue<S, Rt> {
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
    NewTx {
        credential_id: CredentialId,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        tx_nonce: u64,
        original_tx_queue_id: u64,
        result_sender: oneshot::Sender<TransactionReceiverResult<S, Rt>>,
    },
    TxExecuted {
        credential_id: CredentialId,
        tx_nonce: u64,
        tx_result: TransactionReceiverResult<S, Rt>,
        result_sender: oneshot::Sender<TransactionReceiverResult<S, Rt>>,
    },
    TxTimedOut {
        credential_id: CredentialId,
        tx_nonce: u64,
        tx_hash: TxHash,
    },
}

/// This trait encodes the functionality that the buffer task needs for handling submission of
/// queued transactions.
#[async_trait]
pub trait TxExecutionBackend<S: Spec, Rt: Runtime<S>>: Clone {
    fn get_current_nonce_for_user(&self, credential_id: &CredentialId) -> u64;
    async fn execute_tx(
        &self,
        baked_tx: &FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        reason: &'static str,
    ) -> TransactionReceiverResult<S, Rt>;
}

/// The Sequencer backend is a standard implementation when the nonce buffer is used in the
/// preferred sequencer, and allows the sequencer to delegate executing transactions for submission
/// to the queue.
pub struct SequencerTxExecutionBackend<S: Spec, Rt: Runtime<S>> {
    pub api_state: ApiState<S>,
    pub state_updator: Arc<SequencerStateUpdator<S, Rt>>,
}

impl<S: Spec, Rt: Runtime<S>> Clone for SequencerTxExecutionBackend<S, Rt> {
    fn clone(&self) -> Self {
        Self {
            api_state: self.api_state.clone(),
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

    async fn execute_tx(
        &self,
        baked_tx: &FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        reason: &'static str,
    ) -> TransactionReceiverResult<S, Rt> {
        self.state_updator
            .accept_tx_msg(baked_tx, tx_hash, original_tx_queue_id, reason)
            .await
    }
}

#[derive(derivative::Derivative)]
#[derivative(Clone(bound = ""))]
pub struct NonceBufferInputSender<E: TxExecutionBackend<S, Rt>, S: Spec, Rt: Runtime<S>> {
    buffer_sender_channel: mpsc::Sender<NonceBufferInput<S, Rt>>,
    execution_backend: E,
}

pub struct NonceBufferTask<E: TxExecutionBackend<S, Rt>, S: Spec, Rt: Runtime<S>> {
    buffers: HashMap<CredentialId, AddressQueue<S, Rt>>,
    buffer_input: mpsc::Receiver<NonceBufferInput<S, Rt>>,
    input_sender: NonceBufferInputSender<E, S, Rt>,
    execution_backend: E,
    maximum_future_nonce_delta: u64,
    future_nonce_transaction_timeout_millis: u64,
}

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
    fn schedule_timeout(&self, credential_id: CredentialId, tx_nonce: u64, tx_hash: TxHash) {
        let input_sender = self.input_sender.clone();
        let timeout = self.future_nonce_transaction_timeout_millis;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(timeout)).await;
            let _ = input_sender
                .buffer_sender_channel
                .send(NonceBufferInput::TxTimedOut {
                    credential_id,
                    tx_nonce,
                    tx_hash,
                })
                .await;
        });
    }

    async fn run(&mut self) {
        while let Some(input) = self.buffer_input.recv().await {
            println!("Nonce buffer task processing input {input:?}");
            match input {
                NonceBufferInput::NewTx {
                    credential_id,
                    baked_tx,
                    tx_hash,
                    tx_nonce,
                    original_tx_queue_id,
                    result_sender,
                } => {
                    let queue = self.buffers.entry(credential_id).or_default();

                    let user_nonce = queue.non_persisted.user_nonce().unwrap_or_else(|| {
                        self.execution_backend
                            .get_current_nonce_for_user(&credential_id)
                    });

                    match determine_action(tx_nonce, user_nonce, self.maximum_future_nonce_delta) {
                        Action::Enqueue => {
                            let old_tx = queue.txs.insert(
                                tx_nonce,
                                QueuedTx {
                                    baked_tx,
                                    tx_hash,
                                    original_tx_queue_id,
                                    nonce_when_queued: user_nonce,
                                    queued_at: Instant::now(),
                                    result_sender,
                                },
                            );
                            if let Some(old_tx) = old_tx {
                                let _ = old_tx.result_sender.send(err_invalid_nonce::<S, Rt>(
                                    tx_hash,
                                    tx_nonce,
                                    user_nonce,
                                    old_tx.nonce_when_queued,
                                    old_tx.queued_at,
                                    credential_id,
                                    InvalidNonceReason::Replaced,
                                ));
                            }
                            self.schedule_timeout(credential_id, tx_nonce, tx_hash);
                        }
                        Action::Execute => {
                            queue.non_persisted.mark_inflight(tx_nonce);
                            let input_sender = self.input_sender.clone();
                            let backend = self.execution_backend.clone();
                            tokio::spawn(async move {
                                let tx_result = backend
                                    .execute_tx(
                                        &baked_tx,
                                        tx_hash,
                                        original_tx_queue_id,
                                        "nonce_queue_immediate",
                                    )
                                    .await;
                                let _ = input_sender
                                    .buffer_sender_channel
                                    .send(NonceBufferInput::TxExecuted {
                                        credential_id,
                                        tx_nonce,
                                        tx_result,
                                        result_sender,
                                    })
                                    .await;
                            });
                        }
                        Action::Reject => {
                            let _ = result_sender.send(err_invalid_nonce::<S, Rt>(
                                tx_hash,
                                tx_nonce,
                                user_nonce,
                                user_nonce,
                                Instant::now(),
                                credential_id,
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
                    let queue = self.buffers.entry(credential_id).or_default();
                    if tx_result.as_ref().is_ok_and(|r| r.is_ok()) {
                        queue
                            .non_persisted
                            .mark_inflight_execution_succeeded(tx_nonce);
                    } else {
                        queue.non_persisted.mark_inflight_execution_failed(tx_nonce);
                    }

                    let _ = result_sender.send(tx_result); // If the receiver was dropped, ignore

                    let user_nonce = queue
                        .non_persisted
                        .user_nonce()
                        .expect("We just marked the nonce on the queue, so it cannot be None");
                    println!("\nAfter execution, next nonce being queued is {user_nonce}");
                    loop {
                        let Some(head_entry) = queue.txs.first_entry() else {
                            break;
                        };
                        match head_entry.key().cmp(&user_nonce) {
                            Ordering::Less => {
                                // Stale transaction in queue - evict and ignore.
                                // This should not normally happen either, but we handle it to avoid a deadlock
                                // if it does happen for any reason.
                                head_entry.remove();
                                continue;
                            }
                            Ordering::Greater => {
                                // First transaction starts in the future - nothing ready to execute yet
                                break;
                            }
                            Ordering::Equal => {
                                // First transaction is the next expected nonce. Pop it and send
                                // as a NewTx.
                                let tx = head_entry.remove();
                                queue.non_persisted.mark_inflight(user_nonce);
                                let _ = self
                                    .input_sender
                                    .buffer_sender_channel
                                    .send(NonceBufferInput::NewTx {
                                        credential_id,
                                        baked_tx: tx.baked_tx,
                                        tx_hash: tx.tx_hash,
                                        tx_nonce: user_nonce,
                                        original_tx_queue_id: tx.original_tx_queue_id,
                                        result_sender: tx.result_sender,
                                    })
                                    .await;
                            }
                        }
                    }
                }
                NonceBufferInput::TxTimedOut {
                    credential_id,
                    tx_nonce,
                    tx_hash,
                } => {
                    let queue = self.buffers.entry(credential_id).or_default();
                    let state_nonce = self
                        .execution_backend
                        .get_current_nonce_for_user(&credential_id);
                    let user_nonce_for_prerequisites = queue
                        .non_persisted
                        .user_nonce_to_use_as_prerequisite_start()
                        .unwrap_or(state_nonce);
                    if queue.has_contiguity_between(user_nonce_for_prerequisites, tx_nonce) {
                        self.schedule_timeout(credential_id, tx_nonce, tx_hash);
                    } else {
                        match queue.txs.entry(tx_nonce) {
                            // If the hash doesn't match, it means the tx has been replaced. The
                            // `result_sender` is for the new tx and we shouldn't notify it.
                            Entry::Occupied(entry) if entry.get().tx_hash == tx_hash => {
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
                }
            }
        }
    }

    pub fn spawn(
        execution_backend: E,
        maximum_future_nonce_delta: u64,
        future_nonce_transaction_timeout_millis: u64,
    ) -> (JoinHandle<()>, NonceBufferInputSender<E, S, Rt>) {
        let (buffer_sender_channel, buffer_input) = mpsc::channel(MAX_BUFFERED_TXS);
        let input_sender = NonceBufferInputSender {
            buffer_sender_channel,
            execution_backend: execution_backend.clone(),
        };
        let mut task = NonceBufferTask {
            buffers: Default::default(),
            buffer_input,
            input_sender: input_sender.clone(),
            execution_backend,
            maximum_future_nonce_delta,
            future_nonce_transaction_timeout_millis,
        };

        let handle = tokio::spawn(async move { task.run().await });
        (handle, input_sender)
    }
}

impl<E: TxExecutionBackend<S, Rt> + Send + 'static, S: Spec, Rt: Runtime<S>>
    NonceBufferInputSender<E, S, Rt>
{
    pub async fn handle_new_tx(
        &self,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        tx_nonce: u64,
        credential_id: CredentialId,
        original_tx_queue_id: u64,
    ) -> TransactionReceiverResult<S, Rt> {
        let (queue_sender, queue_receiver) = oneshot::channel();
        self.buffer_sender_channel.send(NonceBufferInput::NewTx {
            credential_id,
            baked_tx,
            tx_hash,
            tx_nonce,
            original_tx_queue_id,
            result_sender: queue_sender,
        }).await.map_err(|e| {
            tracing::warn!("Sequencer nonce buffer input receiver dropped. Assuming sequencer shutdown. Error: {e:?}");
            SequencerStateUpdatorError::Shutdown
        })?;
        let nonce_when_queued = self
            .execution_backend
            .get_current_nonce_for_user(&credential_id);
        let queued_at = Instant::now();
        queue_receiver.await.unwrap_or_else(|_| {
            // The oneshot sender was dropped. This should normally only happen
            // on shutdown, or if there's a bug.
            let current_nonce = self
                .execution_backend
                .get_current_nonce_for_user(&credential_id);
            err_invalid_nonce::<S, Rt>(
                tx_hash,
                tx_nonce,
                current_nonce,
                nonce_when_queued,
                queued_at,
                credential_id,
                InvalidNonceReason::EvictedBeforeExecution,
            )
        })
    }
}

/// Helper enum for error logging when the nonce queue rejects a transaction.
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
        InvalidNonceReason::Timeout => format!("{was_queued_msg} In that time, the sequencer did not receive all the transactions leading up to this tx's nonce, so it has timed out and is being evicted from the queue."),
        InvalidNonceReason::EvictedBeforeExecution => format!("{was_queued_msg} It was now dropped from the queue for an unknown reason. This should normally only happen when the sequencer is shutting down."),
        InvalidNonceReason::Replaced => format!("{was_queued_msg} But a new transaction with the same nonce has arrived and replaced it in the account's queue."),
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
