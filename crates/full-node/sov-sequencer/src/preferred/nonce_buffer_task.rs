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
        let res = match self.has_in_flight {
            true => Some(
                self.user_nonce()
                    .unwrap_or(0)
                    .checked_add(1)
                    .expect("Overflow adding 1 to user nonce"),
            ),
            false => self.user_nonce(),
        };
        println!("NonPersisted: self is {self:?}; got prereq start nonce {res:?}");
        res
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
        self.last_successfully_executed = tx_nonce_check.checked_sub(1);
    }
}

#[derive(Default)]
struct AddressQueue<S: Spec, Rt: Runtime<S>> {
    txs: BTreeMap<u64, QueuedTx<S, Rt>>,
    non_persisted: NonPersistedTxs,
}

impl<S: Spec, Rt: Runtime<S>> AddressQueue<S, Rt> {
    fn has_contiguity_between(&self, starting_nonce: u64, tx_nonce: u64) -> bool {
        println!("Checking contiguity between starting {starting_nonce} and tx {tx_nonce}");
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
                    println!("User nonce during NewTx: {user_nonce}, for tx nonce: {tx_nonce}");

                    match determine_action(tx_nonce, user_nonce, self.maximum_future_nonce_delta) {
                        Action::Enqueue => {
                            println!("Determined action: Enqueue (for nonce {tx_nonce}, current user nonce: {user_nonce})");
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
                            println!("Determined action: Execute (for nonce {tx_nonce}, current user nonce: {user_nonce})");
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
                            println!("Determined action: Reject (for nonce {tx_nonce}, current user nonce: {user_nonce})");
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
                        .unwrap_or(0); // The only way the nonce can be Nonce is if we called
                                       // mark_inflight_execution_failed(), it tried to set
                                       // last_executed to tx_nonce.checked_sub(1) but tx_nonce was
                                       // 0.
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

    const DEFAULT_TEST_MAX_QUEUE_SIZE: u64 = 100;
    const DEFAULT_TEST_QUEUE_TIMEOUT_MS: u64 = 100; // 100ms timeout should be enough for sync txs to execute without delaying unit tests much

    // Helper to create a mock QueuedTx for testing
    fn create_mock_queued_tx(nonce: u8) -> QueuedTx<TestSpec, TestRuntime> {
        create_mock_queued_tx_with_hash(nonce, [nonce; 32])
    }

    fn create_mock_queued_tx_with_hash(nonce: u8, hash: [u8; 32]) -> QueuedTx<TestSpec, TestRuntime> {
        let (sender, _receiver) = oneshot::channel();
        QueuedTx {
            baked_tx: FullyBakedTx {
                // Hacky fake data to help track nonces when TXs are sent through the queue
                data: vec![nonce].into(),
            },
            tx_hash: TxHash::from(hash),
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

    /// Mock implementation of TxExecutionBackend for testing handle_new_tx
    #[derive(Clone)]
    struct MockTxExecutionBackend {
        api_nonce: Arc<AtomicU64>,
        executor_nonce: Arc<AtomicU64>,
        executed_txs: Arc<Mutex<Vec<(TxHash, u64)>>>,
        should_fail_nonce: Arc<Mutex<Option<u64>>>,
        execution_delay: Duration,
        db_delay: Duration,
    }

    #[allow(dead_code)]
    impl MockTxExecutionBackend {
        fn new() -> Self {
            Self {
                api_nonce: Arc::new(AtomicU64::new(0)),
                executor_nonce: Arc::new(AtomicU64::new(0)),
                executed_txs: Arc::new(Mutex::new(Vec::new())),
                should_fail_nonce: Arc::new(Mutex::new(None)),
                execution_delay: Duration::from_millis(0),
                db_delay: Duration::from_millis(0),
            }
        }

        fn with_current_nonce(self, nonce: u64) -> Self {
            self.api_nonce.store(nonce, Ordering::SeqCst);
            self.executor_nonce.store(nonce, Ordering::SeqCst);
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

        async fn execute_tx(
            &self,
            baked_tx: &FullyBakedTx,
            tx_hash: TxHash,
            _original_tx_queue_id: u64,
            _reason: &'static str,
        ) -> TransactionReceiverResult<TestSpec, TestRuntime> {
            // Simulate execution delay (state transition time)
            if !self.execution_delay.is_zero() {
                println!("TEST BACKEND sleeping for {}", self.execution_delay.as_millis());
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
                    }
                });
            });

            Ok(Ok(rx))
        }
    }

    /// Helper to create a default task for tests that don't care about specific config
    /// Returns (sender, backend, _shutdown_sender) - the shutdown sender must be kept alive
    /// for the duration of the test or the task will exit immediately.
    fn test_buffer_task(backend: &MockTxExecutionBackend, timeout_override: Option<u64>) -> (
        NonceBufferInputSender<MockTxExecutionBackend, TestSpec, TestRuntime>,
        watch::Sender<()>,
    ) {
        let (shutdown_sender, shutdown_receiver) = watch::channel(());
        let (_handle, sender) = NonceBufferTask::spawn(
            backend.clone(),
            DEFAULT_TEST_MAX_QUEUE_SIZE,
            timeout_override.unwrap_or(DEFAULT_TEST_QUEUE_TIMEOUT_MS),
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
        hash: [u8; 32]
    ) -> JoinHandle<TransactionReceiverResult<TestSpec, TestRuntime>> {
        let to_queue = create_mock_queued_tx_with_hash(nonce, hash);
        let handle = tokio::spawn(async move {
            sender
                .handle_new_tx(
                    to_queue.baked_tx,
                    to_queue.tx_hash,
                    nonce.into(),
                    CredentialId::from_bytes([1u8; 32]),
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
        StfNonceReject(u8, u8),
    }

    async fn assert_on_results(results: Vec<TransactionReceiverResult<TestSpec, TestRuntime>>, expected_outcomes: Vec<Outcome>) {
        let mut results = results.into_iter();
        let expected_outcomes = expected_outcomes.into_iter();

        for (i, outcome) in expected_outcomes.enumerate() {
            let result = results.next().expect("Test passed more expected outcomes than there were tx results");
            match outcome {
                Outcome::Ok => assert!(result.unwrap().unwrap().await.is_ok(), "Expected tx {i} to succeed"),
                Outcome::Err(reason) => {
                    let inner = result.expect("Expected Ok from TransactionReceiverResult");
                    let err = inner.expect_err(&format!("Expected tx {i} to fail with nonce error"));

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
                        other => panic!("Tx {i}: Expected UnsuccessfulTransaction error, got: {other:?}"),
                    }
                },
                Outcome::Revert => {
                    let inner = result.expect("Expected Ok from TransactionReceiverResult");
                    let err = inner.expect_err(&format!("Expected tx {i} to fail with rejection"));

                    assert!(
                        matches!(
                            err,
                            AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                                RollupBlockExecutorError::UnsuccessfulTransaction {
                                    receipt: TransactionReceipt {
                                        receipt: sov_rollup_interface::stf::TxEffect::Skipped(SkippedTxContents {
                                            error: TxProcessingError::RejectedByPreFlight,
                                            ..
                                        }),
                                        ..
                                    }
                                },
                            ))
                        ),
                        "Tx {i}: Expected RejectedByPreFlight rejection, got: {err:?}"
                    );
                },
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
        };
        assert!(results.next().is_none(), "Test passed more tx results than there were expected outcomes")
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
        assert_on_results(results, vec![Outcome::Err(InvalidNonceReason::Invalid), Outcome::Err(InvalidNonceReason::Invalid), Outcome::Ok]).await;
        assert_eq!(get_test_nonce(&backend), 6);
    }

    #[tokio::test]
    async fn test_rejects_too_far_future_nonce() {
        let (backend, sender, _shutdown) = default_test_buffer_task();

        let handles = submit_transactions(sender, vec![DEFAULT_TEST_MAX_QUEUE_SIZE as u8 + 1, DEFAULT_TEST_MAX_QUEUE_SIZE as u8]).await;
        let results = collect_results(handles).await;

        assert!(backend.get_executed_nonces().is_empty());
        // First tx should have been rejected as invalid. Second one should be right at the limit
        // and so should have been queued (and timed out)
        assert_on_results(results, vec![Outcome::Err(InvalidNonceReason::Invalid), Outcome::Err(InvalidNonceReason::Timeout)]).await;
        assert_eq!(get_test_nonce(&backend), 0);
    }

    #[tokio::test]
    async fn test_execution_stops_at_gap_and_times_out() {
        let (backend, sender, _shutdown) = default_test_buffer_task();

        let handles = submit_transactions(sender, vec![0, 1, 3, 4]).await;
        let results = collect_results(handles).await;

        // Should have executed exactly 2 transactions (0 and 1), then stopped at gap
        assert_on_results(results, vec![Outcome::Ok, Outcome::Ok, Outcome::Err(InvalidNonceReason::Timeout), Outcome::Err(InvalidNonceReason::Timeout)]).await;
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

    #[tokio::test]
    async fn test_queued_tx_timeout_loops_if_all_prerequisites_present() {
        // Timeout longer than execution delay but shorter than total time to execute all
        // transactions, to ensure we hit the timeout loop
        let backend = MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(200));
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

        let result_0 = submit_single_transaction(sender.clone(), 0).await.await.expect("JoinHandle panicked");
        // At this point tx 0 has been sent to the buffer and is executing
        // We submit tx 1 immediately and verify it isn't rejected and doesn't timeout
        let result_1 = submit_single_transaction(sender, 1).await.await.expect("JoinHandle panicked");
        assert_on_results(vec![result_0, result_1], vec![Outcome::Ok, Outcome::Ok]).await;

        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_non_persisted_nonce_tracking_slow_execution() {
        // This test verifies that the queue avoids evicting transactions if a current one is
        // executing. Similar to the db timeout test.
        // - Tx 0 executes slowly
        // - Tx 1 arrives mid-execution. The queue should A) not queue it for execution yet - maybe
        // tx 0 will fail. But also B) not evict it - if tx 0 succeeds then tx 1 can be executed.
        let backend = MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(500)); // Slow execution
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

    #[tokio::test]
    async fn test_non_persisted_nonce_tracking_slow_execution_with_rejection() {
        // This test verifies correct behaviour if a transaction arrives while the previous one is
        // in-flight, and then the previous one rejects.
        // - Tx 0 executes slowly
        // - Tx 1 arrives mid-execution. The queue should not queue it for execution yet - maybe
        // tx 0 will fail. Once tx 0 fails, then tx 1 will time out.
        let backend = MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(500)).with_failure_at_nonce(0);
        let (sender, _shutdown) = test_buffer_task(&backend, Some(200)); // Queue times out faster than execution

        let handle_0 = submit_single_transaction(sender.clone(), 0).await;
        // At this point transaction 0 is executing slowly
        // We submit tx 1 immediately and verify it isn't rejected (as it would be if the queue
        // submitted it to the STF), but rather times out
        let handle_1 = submit_single_transaction(sender, 1).await;
        let results = collect_results(vec![handle_0, handle_1]).await;
        assert_on_results(results, vec![Outcome::Revert, Outcome::Err(InvalidNonceReason::Timeout)]).await;
        assert!(backend.get_executed_nonces().is_empty());
        assert_eq!(get_test_nonce(&backend), 0);
    }

    #[tokio::test]
    async fn test_replacement_of_queued_transaction() {
        let backend = MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(200));
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
            vec![Outcome::Ok, Outcome::Err(InvalidNonceReason::Replaced), Outcome::Ok],
        ).await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }

    #[tokio::test]
    async fn test_replacement_of_inflight_transaction_success() {
        let backend = MockTxExecutionBackend::new().with_execution_delay(Duration::from_millis(300));
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
        ).await;
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
        assert_on_results(
            results,
            vec![Outcome::Revert, Outcome::Ok, Outcome::Ok],
        ).await;
        assert_eq!(backend.get_executed_nonces(), vec![0, 1]);
        assert_eq!(get_test_nonce(&backend), 2);
    }
}
