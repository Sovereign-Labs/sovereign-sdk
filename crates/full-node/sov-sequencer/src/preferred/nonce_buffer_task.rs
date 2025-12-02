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
use std::collections::btree_map;
use std::collections::hash_map;
use std::collections::HashMap;
use std::collections::{btree_map::OccupiedEntry, BTreeMap};
use std::fmt::Debug;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch};
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
            tracing::error!("Sequencer nonce buffer: non-persisted tracking: attempted to execute nonce that does not match tracked next nonce!");
        } else if self.last_successfully_executed.is_none() {
            // If we're marking a transction as in-flight, that means we definitely know the
            // previous one has been executed. Probably from the API state.
            self.last_successfully_executed = tx_nonce_check.checked_sub(1);
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
        }
        self.has_in_flight = false;

        self.last_successfully_executed = self.last_successfully_executed.map(|last| {
            let new = last.checked_add(1).expect("Overflow adding 1 to user nonce");
            if new != tx_nonce_check {
                tracing::error!("Sequencer nonce buffer: non-persisted tracking: after executing, incremented nonce did not match tx nonce!");
            }
            new
        }).or(Some(tx_nonce_check));
    }

    fn mark_inflight_execution_failed(&mut self, tx_nonce_check: u64) {
        if !self.has_in_flight {
            tracing::error!("Sequencer nonce buffer: non-persisted tracking: attempted to mark executed tx successful when has_in_flight is false!");
        }
        self.has_in_flight = false;

        if self.last_successfully_executed.is_some_and(|l| {
            l.checked_add(1).expect("Overflow adding 1 to user nonce") != tx_nonce_check
        }) {
            tracing::error!("Sequencer nonce buffer: non-persisted tracking: marked failed to execute, tx whose nonce was non-consecutive with last known one");
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
    /// A tx to be executed. Either newly arrived from the API, or newly valid and popped from the
    /// head of the queue.
    NewTx {
        credential_id: CredentialId,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
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
    },
    // Tx has been waiting in the queue until the timeout has hit. If the pre-requisite nonces have
    // not been queued yet, it may get evicted now.
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
                    println!("User nonce during NewTx: {user_nonce}");

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
                            println!("Determined action: Execute (for nonce {tx_nonce})");
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
                            println!("Determined action: Execute (for nonce {tx_nonce})");
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
                NonceBufferInput::TxPersisted { credential_id } => {
                    let hash_map::Entry::Occupied(entry) = self.buffers.entry(credential_id) else {
                        continue;
                    };
                    if entry.get().txs.is_empty() {
                        let real_nonce = self
                            .execution_backend
                            .get_current_nonce_for_user(&credential_id);
                        if entry
                            .get()
                            .non_persisted
                            .user_nonce_to_use_as_prerequisite_start()
                            .is_none_or(|n| n <= real_nonce)
                        {
                            entry.remove();
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
                }
            }
        }
    }

    pub fn spawn(
        execution_backend: E,
        maximum_future_nonce_delta: u64,
        future_nonce_transaction_timeout_millis: u64,
        mut shutdown_receiver: watch::Receiver<()>,
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

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = task.run() => {},
                _ = shutdown_receiver.changed() => {}

            }
        });
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

    pub async fn mark_tx_persisted(&self, credential_id: CredentialId) {
        let _ = self
            .buffer_sender_channel
            .send(NonceBufferInput::TxPersisted { credential_id })
            .await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferred::{DoNewTxError, RollupBlockExecutorError};
    use sov_modules_api::{SkippedTxContents, TransactionReceipt, TxProcessingError};
    use sov_test_utils::runtime::TestOptimisticRuntime;
    use sov_test_utils::TestSpec;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::oneshot;

    type TestRuntime = TestOptimisticRuntime<TestSpec>;

    // Helper to create a mock QueuedTx for testing
    fn create_mock_queued_tx(nonce: u8) -> QueuedTx<TestSpec, TestRuntime> {
        let (sender, _receiver) = oneshot::channel();
        QueuedTx {
            baked_tx: FullyBakedTx {
                // Hacky fake data to help track nonces when TXs are sent through the queue
                data: vec![nonce].into(),
            },
            tx_hash: TxHash::from([0u8; 32]),
            nonce_when_queued: 0,
            queued_at: Instant::now(),
            original_tx_queue_id: 0,
            result_sender: sender,
        }
    }

    fn nonce_from_queued_tx(tx: &QueuedTx<TestSpec, TestRuntime>) -> u8 {
        *tx.baked_tx.data.first().unwrap()
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
        queue.non_persisted.mark_inflight_execution_failed(0);
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

    fn create_default_confirmation() -> Confirmation<TestSpec, TestRuntime> {
        Confirmation {
            events: vec![],
            receipt: sov_modules_api::ApiTxEffect::Skipped {
                data: SkippedTxContents {
                    gas_used: <TestSpec as Spec>::Gas::from([0, 0]),
                    error: TxProcessingError::RejectedByPreFlight,
                },
            },
            tx_number: 0,
        }
    }

    /// Mock implementation of TxExecutionBackend for testing handle_new_tx
    #[derive(Clone)]
    struct MockTxExecutionBackend {
        current_nonce: Arc<AtomicU64>,
        executed_txs: Arc<Mutex<Vec<(TxHash, u64)>>>,
        should_fail_nonce: Option<u64>,
        execution_delay: Duration,
        db_delay: Duration,
    }

    #[allow(dead_code)]
    impl MockTxExecutionBackend {
        fn new() -> Self {
            Self {
                current_nonce: Arc::new(AtomicU64::new(0)),
                executed_txs: Arc::new(Mutex::new(Vec::new())),
                should_fail_nonce: None,
                execution_delay: Duration::from_millis(0),
                db_delay: Duration::from_millis(0),
            }
        }

        fn with_current_nonce(self, nonce: u64) -> Self {
            self.current_nonce.store(nonce, Ordering::SeqCst);
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

        fn with_failure_at_nonce(mut self, nonce: u64) -> Self {
            self.should_fail_nonce = Some(nonce);
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

        fn set_nonce(&self, new_nonce: u64) {
            self.current_nonce.store(new_nonce, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl TxExecutionBackend<TestSpec, TestRuntime> for MockTxExecutionBackend {
        fn get_current_nonce_for_user(&self, _credential_id: &CredentialId) -> u64 {
            self.current_nonce.load(Ordering::SeqCst)
        }

        async fn execute_tx(
            &self,
            baked_tx: &FullyBakedTx,
            tx_hash: TxHash,
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
            if self.should_fail_nonce == Some(tx_nonce) {
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

            // Track execution
            self.executed_txs.lock().unwrap().push((tx_hash, tx_nonce));

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
                backend.set_nonce(tx_nonce + 1);

                // Send result
                let _ = tx.send(AcceptedTx::<Confirmation<TestSpec, TestRuntime>> {
                    tx: baked_tx_clone,
                    tx_hash,
                    confirmation: create_default_confirmation(),
                });
            });

            Ok(Ok(rx))
        }
    }

    fn create_test_tx(nonce: u8) -> FullyBakedTx {
        FullyBakedTx {
            // Use nonce as data to track it
            data: vec![nonce].into(),
        }
    }

    /// Helper to create a default task for tests that don't care about specific config
    fn default_test_buffer_task() -> (
        NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        MockTxExecutionBackend,
    ) {
        let backend = MockTxExecutionBackend::new();
        let (_shutdown_sender, shutdown_receiver) = watch::channel(());
        let (_handle, sender) = NonceBufferTask::spawn(
            backend.clone(),
            100,
            1000, // 100ms timeout should be enough for sync txs to execute without delaying unit tests much
            shutdown_receiver,
        );
        (sender, backend)
    }

    async fn enqueue_mock_queued_tx(
        sender: NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        nonce: u8,
    ) -> TransactionReceiverResult<TestSpec, TestRuntime> {
        let to_queue = create_mock_queued_tx(nonce);
        sender
            .handle_new_tx(
                to_queue.baked_tx,
                to_queue.tx_hash,
                nonce.into(),
                CredentialId::from_bytes([1u8; 32]),
                to_queue.original_tx_queue_id,
            )
            .await
    }

    #[tokio::test]
    async fn test_drain_evicts_stale_and_executes_ready() {
        let (sender, backend) = default_test_buffer_task();
        let backend = backend.with_current_nonce(5);
        let credential_id = CredentialId::from([1u8; 32]);

        // Enqueue transactions with nonces 3, 4, 5
        let mut handles = vec![];
        for nonce in [3, 4, 5] {
            handles.push(enqueue_mock_queued_tx(sender.clone(), nonce));
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Await all handles and collect results
        let results: Vec<_> = futures::future::join_all(handles).await;

        let executed_nonces = backend.get_executed_nonces();
        assert_eq!(
            executed_nonces,
            vec![5],
            "Should execute only nonce 5, not stale nonces 3 and 4"
        );

        let mut results = results.into_iter();
        assert!(results.next().unwrap().is_err());
        assert!(results.next().unwrap().is_err());
        assert!(results.next().unwrap().unwrap().unwrap().await.is_ok());

        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 6);
    }

    // #[tokio::test]
    // async fn test_drain_stops_at_gap() {
    //     let (queues, backend) = default_test_tx_nonce_queues();
    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Enqueue transactions: 0, 1, 3, 4 (gap at 2)
    //     for nonce in [0, 1, 3, 4] {
    //         let entry = queues.lock_for_address(credential_id);
    //         enqueue_mock_queued_tx(entry, nonce);
    //     }

    //     // Drain starting from nonce 0
    //     queues.drain_any_ready_transactions(&credential_id, 0).await;

    //     // Should have executed exactly 2 transactions (0 and 1), then stopped at gap
    //     let executed_nonces = backend.get_executed_nonces();
    //     assert_eq!(
    //         executed_nonces,
    //         vec![0, 1],
    //         "Should execute nonces 0 and 1, then stop at gap (missing nonce 2)"
    //     );

    //     // Transactions 3 and 4 should still be in queue
    //     assert!(queues.has_prerequisites_to_nonce(&credential_id, 4, 3));
    // }

    // #[tokio::test]
    // async fn test_immediate_execution_triggers_drain() {
    //     let backend = MockTxExecutionBackend::new().with_current_nonce(0);
    //     let queues = TxNonceQueues::new(
    //         backend.clone(),
    //         10,   // max_future_nonce_delta
    //         5000, // timeout (long enough to not trigger)
    //     );

    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TXs with nonces 1, 2, 3 (should be queued)
    //     let mut handles = vec![];
    //     for nonce in 1u8..=3 {
    //         let queues = queues.clone();
    //         let handle = tokio::spawn(async move {
    //             queues
    //                 .handle_new_tx(
    //                     create_test_tx(nonce),
    //                     TxHash::from([nonce; 32]),
    //                     nonce as u64,
    //                     credential_id,
    //                     0,
    //                 )
    //                 .await
    //         });
    //         handles.push(handle);
    //     }

    //     // Give queued TXs time to settle
    //     tokio::time::sleep(Duration::from_millis(50)).await;

    //     // Submit TX with nonce 0 (current nonce) - should execute immediately and trigger drain
    //     let result = queues
    //         .handle_new_tx(
    //             create_test_tx(0),
    //             TxHash::from([0; 32]),
    //             0,
    //             credential_id,
    //             0,
    //         )
    //         .await;

    //     // TX 0 should execute successfully
    //     assert!(result.is_ok());
    //     let inner = result.unwrap();
    //     assert!(inner.is_ok());
    //     let rx = inner.unwrap();
    //     assert!(rx.await.is_ok());

    //     // Wait for all queued TXs to complete
    //     for handle in handles {
    //         let result = handle.await.unwrap();
    //         assert!(result.is_ok());
    //         let inner = result.unwrap();
    //         assert!(inner.is_ok());
    //         let rx = inner.unwrap();
    //         assert!(rx.await.is_ok());
    //     }

    //     // Verify all 4 TXs executed in order
    //     let executed = backend.get_executed_nonces();
    //     assert_eq!(
    //         executed,
    //         vec![0, 1, 2, 3],
    //         "All transactions should execute in order after filling the gap"
    //     );

    //     // Verify current nonce advanced
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 4);
    // }

    // #[tokio::test]
    // async fn test_reject_past_nonce() {
    //     let backend = MockTxExecutionBackend::new().with_current_nonce(5);
    //     let queues = TxNonceQueues::new(backend.clone(), 10, 5000);

    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TX with nonce 4 (past nonce, current is 5)
    //     let result = queues
    //         .handle_new_tx(
    //             create_test_tx(4),
    //             TxHash::from([4; 32]),
    //             4,
    //             credential_id,
    //             0,
    //         )
    //         .await;

    //     // Should get an error result
    //     assert!(result.is_ok());
    //     let inner = result.unwrap();
    //     assert!(inner.is_err());

    //     match inner.unwrap_err() {
    //         AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
    //             RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
    //         )) => {
    //             // Verify it's a nonce error
    //             match receipt.receipt {
    //                 sov_rollup_interface::stf::TxEffect::Skipped(contents) => {
    //                     match contents.error {
    //                         TxProcessingError::CheckUniquenessFailed(msg) => {
    //                             assert!(
    //                                 msg.contains("bad nonce"),
    //                                 "Error should mention bad nonce: \"{msg}\"",
    //                             );
    //                             assert!(
    //                                 msg.contains("expected: 5"),
    //                                 "Error should mention expected nonce 5: \"{msg}\"",
    //                             );
    //                             assert!(
    //                                 msg.contains("found: 4"),
    //                                 "Error should mention found nonce 4: \"{msg}\"",
    //                             );
    //                         }
    //                         _ => panic!("Expected CheckUniquenessFailed error"),
    //                     }
    //                 }
    //                 _ => panic!("Expected Skipped receipt"),
    //             }
    //         }
    //         _ => panic!("Expected UnsuccessfulTransaction error"),
    //     }

    //     // Verify no TXs were executed
    //     assert!(backend.get_executed_nonces().is_empty());

    //     // Verify nonce didn't change
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 5);
    // }

    // #[tokio::test]
    // async fn test_reject_too_far_future_nonce() {
    //     let backend = MockTxExecutionBackend::new().with_current_nonce(0);
    //     let queues = TxNonceQueues::new(
    //         backend.clone(),
    //         10, // max_future_nonce_delta = 10, so max valid is 0+10 = 10
    //         5000,
    //     );

    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TX with nonce 11 (too far in future, max valid is 10)
    //     let result = queues
    //         .handle_new_tx(
    //             create_test_tx(11),
    //             TxHash::from([11; 32]),
    //             11,
    //             credential_id,
    //             0,
    //         )
    //         .await;

    //     // Should get an error result
    //     assert!(result.is_ok());
    //     let inner = result.unwrap();
    //     assert!(inner.is_err());

    //     match inner.unwrap_err() {
    //         AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
    //             RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
    //         )) => {
    //             // Verify it's a nonce error
    //             match receipt.receipt {
    //                 sov_rollup_interface::stf::TxEffect::Skipped(contents) => {
    //                     match contents.error {
    //                         TxProcessingError::CheckUniquenessFailed(msg) => {
    //                             assert!(
    //                                 msg.contains("bad nonce"),
    //                                 "Error should mention bad nonce: \"{msg}\"",
    //                             );
    //                         }
    //                         _ => panic!("Expected CheckUniquenessFailed error"),
    //                     }
    //                 }
    //                 _ => panic!("Expected Skipped receipt"),
    //             }
    //         }
    //         _ => panic!("Expected UnsuccessfulTransaction error"),
    //     }

    //     // Verify no TXs were executed
    //     assert!(backend.get_executed_nonces().is_empty());

    //     // Verify nonce didn't change
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 0);
    // }

    // #[tokio::test]
    // async fn test_queue_future_nonce_then_fill_gap() {
    //     let backend = MockTxExecutionBackend::new().with_current_nonce(0);
    //     let queues = TxNonceQueues::new(backend.clone(), 10, 5000);
    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TX with nonce 2 (should queue, current is 0)
    //     let handle_2 = tokio::spawn({
    //         let queues = queues.clone();
    //         async move {
    //             queues
    //                 .handle_new_tx(
    //                     create_test_tx(2),
    //                     TxHash::from([2; 32]),
    //                     2,
    //                     credential_id,
    //                     0,
    //                 )
    //                 .await
    //         }
    //     });

    //     // Give it time to queue
    //     tokio::time::sleep(Duration::from_millis(50)).await;

    //     // Submit TX with nonce 0 (should execute immediately, but NOT drain N+2 due to gap at N+1)
    //     let result_0 = queues
    //         .handle_new_tx(
    //             create_test_tx(0),
    //             TxHash::from([0; 32]),
    //             0,
    //             credential_id,
    //             0,
    //         )
    //         .await;
    //     assert!(result_0.is_ok());
    //     let rx_0 = result_0.unwrap().unwrap();
    //     assert!(rx_0.await.is_ok());

    //     // Give drain task time to run (it shouldn't drain N+2)
    //     tokio::time::sleep(Duration::from_millis(100)).await;

    //     // Verify only N=0 executed, N+2 still queued
    //     assert_eq!(backend.get_executed_nonces(), vec![0]);
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 1);

    //     // Now submit TX with nonce 1 (should execute and trigger drain of N+2)
    //     let result_1 = queues
    //         .handle_new_tx(
    //             create_test_tx(1),
    //             TxHash::from([1; 32]),
    //             1,
    //             credential_id,
    //             0,
    //         )
    //         .await;
    //     assert!(result_1.is_ok());
    //     let rx_1 = result_1.unwrap().unwrap();
    //     assert!(rx_1.await.is_ok());

    //     // Wait for N+2 to complete
    //     let result_2 = handle_2.await.unwrap();
    //     assert!(result_2.is_ok());
    //     let rx_2 = result_2.unwrap().unwrap();
    //     assert!(rx_2.await.is_ok());

    //     // Verify all executed in order
    //     assert_eq!(backend.get_executed_nonces(), vec![0, 1, 2]);
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 3);
    // }

    // #[tokio::test]
    // async fn test_queued_tx_timeout_without_prerequisites() {
    //     let backend = MockTxExecutionBackend::new().with_current_nonce(0);
    //     let queues = TxNonceQueues::new(
    //         backend.clone(),
    //         10,
    //         200, // Short timeout (200ms) to make test fast
    //     );
    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TX with nonce 1 (will queue and wait)
    //     let handle = tokio::spawn({
    //         let queues = queues.clone();
    //         async move {
    //             queues
    //                 .handle_new_tx(
    //                     create_test_tx(1),
    //                     TxHash::from([1; 32]),
    //                     1,
    //                     credential_id,
    //                     0,
    //                 )
    //                 .await
    //         }
    //     });

    //     // Wait for timeout to trigger (200ms timeout + some buffer)
    //     tokio::time::sleep(Duration::from_millis(300)).await;

    //     // TX should have been evicted with nonce error
    //     let result = handle.await.unwrap();
    //     assert!(result.is_ok());
    //     let inner = result.unwrap();
    //     assert!(inner.is_err());

    //     // Verify it's a nonce error
    //     match inner.unwrap_err() {
    //         AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
    //             RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
    //         )) => match receipt.receipt {
    //             sov_rollup_interface::stf::TxEffect::Skipped(contents) => match contents.error {
    //                 TxProcessingError::CheckUniquenessFailed(msg) => {
    //                     assert!(
    //                         msg.contains("bad nonce"),
    //                         "Error should mention bad nonce: \"{msg}\"",
    //                     );
    //                     assert!(
    //                         msg.contains("expected: 0"),
    //                         "Error should mention expected nonce 0: \"{msg}\"",
    //                     );
    //                     assert!(
    //                         msg.contains("found: 1"),
    //                         "Error should mention found nonce 1: \"{msg}\"",
    //                     );
    //                 }
    //                 _ => panic!("Expected CheckUniquenessFailed error"),
    //             },
    //             _ => panic!("Expected Skipped receipt"),
    //         },
    //         _ => panic!("Expected UnsuccessfulTransaction error"),
    //     }

    //     // Verify no TXs were executed
    //     assert!(backend.get_executed_nonces().is_empty());
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 0);

    //     // Verify TX was removed from queue
    //     assert!(!queues.has_prerequisites_to_nonce(&credential_id, 1, 0));
    // }

    // #[tokio::test]
    // async fn test_queued_tx_timeout_loops_if_all_prerequisites_present() {
    //     let backend = MockTxExecutionBackend::new()
    //         .with_current_nonce(0)
    //         .with_execution_delay(Duration::from_millis(200)); // Each TX takes 200ms

    //     let queues = TxNonceQueues::new(
    //         backend.clone(),
    //         10,
    //         500, // Timeout is 250ms, longer than execution delay but shorter than total time to
    //              // execute all queued TXs
    //     );
    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TXs with nonces 1-5 (all will queue)
    //     let mut handles = vec![];
    //     for nonce in 1u8..=5 {
    //         let queues = queues.clone();
    //         let handle = tokio::spawn(async move {
    //             queues
    //                 .handle_new_tx(
    //                     create_test_tx(nonce),
    //                     TxHash::from([nonce; 32]),
    //                     nonce as u64,
    //                     credential_id,
    //                     0,
    //                 )
    //                 .await
    //         });
    //         handles.push(handle);
    //     }

    //     // Give TXs time to queue
    //     tokio::time::sleep(Duration::from_millis(50)).await;

    //     // Now submit TX with nonce 0 to trigger drain
    //     let result_0 = queues
    //         .handle_new_tx(
    //             create_test_tx(0),
    //             TxHash::from([0; 32]),
    //             0,
    //             credential_id,
    //             0,
    //         )
    //         .await;
    //     assert!(result_0.is_ok());
    //     let rx_0 = result_0.unwrap().unwrap();
    //     assert!(rx_0.await.is_ok());

    //     // Wait for all queued TXs to complete
    //     // Each TX takes 200ms, so 5 TXs = ~1000ms total
    //     // Multiple timeouts (250ms each) will fire during this time
    //     // but TXs should NOT be evicted because they have prerequisites
    //     for handle in handles {
    //         let result = handle.await.unwrap();
    //         assert!(result.is_ok(), "TX should not timeout - has prerequisites");
    //         let rx = result.unwrap().unwrap();
    //         assert!(rx.await.is_ok());
    //     }

    //     // Verify all TXs executed in order
    //     assert_eq!(
    //         backend.get_executed_nonces(),
    //         vec![0, 1, 2, 3, 4, 5],
    //         "All transactions should execute despite timeouts firing during drain"
    //     );
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 6);
    // }

    // #[tokio::test]
    // async fn test_oneshot_sender_dropped_returns_error() {
    //     let backend = MockTxExecutionBackend::new().with_current_nonce(0);
    //     let queues = TxNonceQueues::new(backend.clone(), 10, 500);
    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TX with nonce 1 (will queue)
    //     let handle = tokio::spawn({
    //         let queues = queues.clone();
    //         async move {
    //             queues
    //                 .handle_new_tx(
    //                     create_test_tx(1),
    //                     TxHash::from([1; 32]),
    //                     1,
    //                     credential_id,
    //                     0,
    //                 )
    //                 .await
    //         }
    //     });

    //     // Give it time to queue
    //     tokio::time::sleep(Duration::from_millis(100)).await;

    //     // Manually evict the TX (drops the oneshot sender)
    //     let evicted = queues.evict(&credential_id, 1);
    //     assert!(evicted.is_some(), "TX should have been queued");

    //     // Add a timeout to detect if the task hangs
    //     let result = tokio::time::timeout(Duration::from_secs(2), handle)
    //         .await
    //         .expect("Task should complete within 2 seconds (sender was dropped)")
    //         .unwrap();

    //     // When the sender is dropped, the receiver errors, which gets mapped to a nonce error
    //     assert!(result.is_ok(), "Should not have sequencer error");
    //     let inner = result.unwrap();

    //     // The inner result should be an error (AcceptTxError with nonce error)
    //     assert!(
    //         inner.is_err(),
    //         "Should have nonce error when sender dropped"
    //     );

    //     // Verify it's a nonce error
    //     match inner.unwrap_err() {
    //         AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
    //             RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
    //         )) => match receipt.receipt {
    //             sov_rollup_interface::stf::TxEffect::Skipped(contents) => match contents.error {
    //                 TxProcessingError::CheckUniquenessFailed(msg) => {
    //                     assert!(
    //                         msg.contains("bad nonce"),
    //                         "Error should mention bad nonce: \"{msg}\"",
    //                     );
    //                 }
    //                 _ => panic!(
    //                     "Expected CheckUniquenessFailed error, got: {:?}",
    //                     contents.error
    //                 ),
    //             },
    //             _ => panic!("Expected Skipped receipt"),
    //         },
    //         other => panic!("Expected nonce error, got: {other:?}"),
    //     }

    //     // Verify no TXs were executed
    //     assert!(backend.get_executed_nonces().is_empty());
    // }

    // #[tokio::test]
    // async fn test_race_condition_with_db_delay() {
    //     // This test verifies that the race condition is fixed:
    //     // - TX A (nonce N) executes successfully, but DB persistence is slow
    //     // - TX B (nonce N+1) arrives before DB completes
    //     // - Without the fix: B would see current_nonce=N and get queued (deadlock!)
    //     // - With the fix: B sees last_popped=N, so current_nonce becomes N+1, and executes immediately

    //     let backend = MockTxExecutionBackend::new()
    //         .with_current_nonce(0)
    //         .with_db_delay(Duration::from_millis(200)); // Slow DB

    //     let queues = TxNonceQueues::new(backend.clone(), 10, 5000);
    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TX A with nonce 0 - should execute immediately
    //     let handle_a = tokio::spawn({
    //         let queues = queues.clone();
    //         async move {
    //             queues
    //                 .handle_new_tx(
    //                     create_test_tx(0),
    //                     TxHash::from([0; 32]),
    //                     0,
    //                     credential_id,
    //                     0,
    //                 )
    //                 .await
    //         }
    //     });

    //     // Wait for execution to complete but NOT for DB to complete
    //     // The execution itself is instant, but DB persistence takes 200ms
    //     tokio::time::sleep(Duration::from_millis(50)).await;

    //     // At this point:
    //     // - TX A has executed (state transition complete)
    //     // - last_popped is set to 0
    //     // - But API state still shows nonce=0 (DB hasn't completed)

    //     // Submit TX B with nonce 1 - should execute immediately because last_popped=0
    //     let handle_b = tokio::spawn({
    //         let queues = queues.clone();
    //         async move {
    //             queues
    //                 .handle_new_tx(
    //                     create_test_tx(1),
    //                     TxHash::from([1; 32]),
    //                     1,
    //                     credential_id,
    //                     0,
    //                 )
    //                 .await
    //         }
    //     });

    //     // Both transactions should succeed
    //     let result_a = handle_a.await.unwrap();
    //     assert!(result_a.is_ok(), "TX A should not have sequencer error");
    //     let rx_a = result_a.unwrap().unwrap();
    //     assert!(rx_a.await.is_ok(), "TX A should succeed");

    //     let result_b = handle_b.await.unwrap();
    //     assert!(result_b.is_ok(), "TX B should not have sequencer error");
    //     let rx_b = result_b.unwrap().unwrap();
    //     assert!(rx_b.await.is_ok(), "TX B should succeed despite DB delay");

    //     // Verify both executed in order
    //     assert_eq!(
    //         backend.get_executed_nonces(),
    //         vec![0, 1],
    //         "Both transactions should execute in order despite DB delay"
    //     );

    //     // Verify final nonce (after DB completes)
    //     assert_eq!(backend.get_current_nonce_for_user(&credential_id), 2);
    // }

    // #[tokio::test]
    // async fn test_queue_cleanup_respects_last_popped() {
    //     // This test verifies that cleanup_queue_if_empty() doesn't prune a queue
    //     // if last_popped is still tracking useful information

    //     let backend = MockTxExecutionBackend::new()
    //         .with_current_nonce(0)
    //         .with_db_delay(Duration::from_millis(100));

    //     let queues = TxNonceQueues::new(backend.clone(), 10, 5000);
    //     let credential_id = CredentialId::from([1u8; 32]);

    //     // Submit TX with nonce 0
    //     let result = queues
    //         .handle_new_tx(
    //             create_test_tx(0),
    //             TxHash::from([0; 32]),
    //             0,
    //             credential_id,
    //             0,
    //         )
    //         .await;
    //     assert!(result.is_ok());

    //     // At this point, last_popped=0 but API state still shows nonce=0 (DB delay)
    //     // The queue should exist (even though it's empty) because last_popped tracks useful info
    //     tokio::time::sleep(Duration::from_millis(50)).await;

    //     // Try to cleanup - should NOT prune because last_popped=0 >= current_nonce=0
    //     queues.cleanup_queue_if_empty(&credential_id);
    //     assert!(
    //         queues.queues.contains_key(&credential_id),
    //         "Queue should not be pruned while last_popped tracks useful info"
    //     );

    //     // Now wait for DB to complete
    //     tokio::time::sleep(Duration::from_millis(100)).await;

    //     // Try cleanup again - now should prune because last_popped=0 < current_nonce=1
    //     queues.mark_completed(&credential_id);
    //     assert!(
    //         !queues.queues.contains_key(&credential_id),
    //         "Queue should be pruned after API state catches up"
    //     );
    // }
}
