use dashmap::DashMap;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::rest::ApiState;
use sov_modules_api::{FullyBakedTx, Runtime, Spec};
use sov_rollup_interface::{crypto::CredentialId, TxHash};
use std::cmp::Ordering;
use std::collections::{btree_map::OccupiedEntry, BTreeMap};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::common::AcceptedTx;

use super::sync_sequencer_state::{
    AcceptTxError, SequencerStateUpdator, SequencerStateUpdatorError,
};
use super::{err_invalid_nonce, Confirmation};

pub(crate) type TransactionReceiverResult<S, Rt> =
    Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>;

pub struct QueuedTx<S: Spec, Rt: Runtime<S>> {
    pub tx: FullyBakedTx,
    pub tx_hash: TxHash,
    pub original_tx_queue_id: u64,
    pub result_sender:
        oneshot::Sender<Result<TransactionReceiverResult<S, Rt>, SequencerStateUpdatorError>>,
}

pub struct AddressQueue<S: Spec, Rt: Runtime<S>> {
    txs: BTreeMap<u64, QueuedTx<S, Rt>>,
    /// Tracks a nonce that has been removed from the queue for execution but hasn't completed yet,
    /// since it's no longer in the queue but it still satisfies the prerequisite for future nonces
    last_executed: Option<u64>,
}

impl<S: Spec, Rt: Runtime<S>> Debug for AddressQueue<S, Rt> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{ nonces: {:?}, last_executed: {:?} }}",
            self.txs.keys().cloned().collect::<Vec<_>>(),
            self.last_executed
        )
    }
}

impl<S: Spec, Rt: Runtime<S>> AddressQueue<S, Rt> {
    fn new() -> Self {
        Self {
            txs: BTreeMap::new(),
            last_executed: None,
        }
    }

    fn head(&mut self) -> Option<OccupiedEntry<u64, QueuedTx<S, Rt>>> {
        self.txs.first_entry()
    }

    fn insert(&mut self, nonce: u64, tx: QueuedTx<S, Rt>) -> Option<QueuedTx<S, Rt>> {
        self.txs.insert(nonce, tx)
    }

    fn remove(&mut self, nonce: u64) -> Option<QueuedTx<S, Rt>> {
        self.txs.remove(&nonce)
    }

    /// Mark a nonce as currently being executed (removed from queue but not yet committed).
    fn mark_executing(&mut self, nonce: u64) {
        self.last_executed = Some(nonce);
    }

    /// Check if there's a contiguous sequence of transactions from `current_nonce` to `tx_nonce` (inclusive).
    /// A nonce is considered "present" if it's either in the queue OR marked as last_executed.
    fn has_contiguous_sequence_to(&self, tx_nonce: u64, current_nonce: u64) -> bool {
        // We know the user account's nonce cannot be lower than the state value.
        // Sometimes the state value is stale for a short period of time, which is why we treat is
        // as a lower bound only.
        let lower_bound = current_nonce;
        // last_executed is the last recorded transaction popped from the user's queue, so we know
        // the account's nonce cannot be higher than this.
        // We add 1 because `current_nonce` is the next valid nonce, while `last_executed` was the
        // previous valid nonce (so the next transaction should have nonce `last_executed + 1`).
        let upper_bound =
            current_nonce.max(self.last_executed.map(|n| n.saturating_add(1)).unwrap_or(0));

        if tx_nonce < lower_bound {
            // The transaction can never be valid.
            false
        } else if lower_bound <= tx_nonce && tx_nonce <= upper_bound {
            // There is uncertainty about the user account's real nonce. In this range, leniently
            // treat transactions as valid (the uncertainty will eventually resolve itself as API
            // state updates).
            true
        } else { // tx_nonce > upper_bound 
            // The transaction's nonce is known to be greater than our upper bound estimate of the
            // next valid nonce.
            // We need to check if there's a contiguous set of transactions actually queued beyond
            // the upper bound.
            let mut expected = upper_bound;
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

    fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }
}

/// This trait encodes the functionality that the TxNonceQueues need for handling submission of
/// queued transactions.
pub trait TxExecutionBackend<S: Spec, Rt: Runtime<S>> {
    fn get_current_nonce_for_user(&self, credential_id: &CredentialId) -> u64;
    fn execute_tx(
        &self,
        baked_tx: &FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        reason: &'static str,
    ) -> impl std::future::Future<
        Output = Result<
            Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>,
            SequencerStateUpdatorError,
        >,
    > + std::marker::Send;
}

/// The Sequencer backend is a standard implementation when the TxNonceQueues object is used in the
/// preferred sequencer, and allows the sequencer to delegate executing transactions for submission
/// to the queue.
/// Doing it this way allows the queue to own a tiny subset of the sequencer's functionality, while
/// the sequencer owns the queue itself.
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
    ) -> Result<
        Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>,
        SequencerStateUpdatorError,
    > {
        self.state_updator
            .accept_tx_msg(baked_tx, tx_hash, original_tx_queue_id, reason)
            .await
    }
}

pub struct TxNonceQueues<Sb: TxExecutionBackend<S, Rt>, S: Spec, Rt: Runtime<S>> {
    queues: Arc<DashMap<CredentialId, AddressQueue<S, Rt>>>,
    submitter: Sb,
    maximum_future_nonce_delta: u64,
    future_nonce_transaction_timeout_millis: u64,
}

impl<Sb: TxExecutionBackend<S, Rt> + Clone, S: Spec, Rt: Runtime<S>> Clone
    for TxNonceQueues<Sb, S, Rt>
{
    fn clone(&self) -> Self {
        Self {
            queues: self.queues.clone(),
            submitter: self.submitter.clone(),
            maximum_future_nonce_delta: self.maximum_future_nonce_delta,
            future_nonce_transaction_timeout_millis: self.future_nonce_transaction_timeout_millis,
        }
    }
}

impl<Sb: TxExecutionBackend<S, Rt> + Sync + Send + Clone + 'static, S: Spec, Rt: Runtime<S>>
    TxNonceQueues<Sb, S, Rt>
{
    pub fn new(
        submitter: Sb,
        maximum_future_nonce_delta: u64,
        future_nonce_transaction_timeout_millis: u64,
    ) -> Self {
        Self {
            queues: Arc::new(DashMap::new()),
            submitter,
            maximum_future_nonce_delta,
            future_nonce_transaction_timeout_millis,
        }
    }

    /// Lock the queue for a specific address for atomic nonce check + enqueue
    pub fn lock_for_address(
        &self,
        credential_id: CredentialId,
    ) -> dashmap::mapref::entry::Entry<CredentialId, AddressQueue<S, Rt>> {
        self.queues.entry(credential_id)
    }

    /// Enqueue a transaction.
    /// Caller needs to hold lock from lock_for_address. This function consumes the Entry holding
    /// the lock inside the map, unlocking it after the function returns.
    pub fn enqueue_and_unlock(
        queue_entry: dashmap::mapref::entry::Entry<CredentialId, AddressQueue<S, Rt>>,
        result_sender: oneshot::Sender<
            Result<TransactionReceiverResult<S, Rt>, SequencerStateUpdatorError>,
        >,
        tx: FullyBakedTx,
        tx_hash: TxHash,
        nonce: u64,
        original_tx_queue_id: u64,
    ) {
        let queued_tx = QueuedTx {
            tx,
            tx_hash,
            original_tx_queue_id,
            result_sender,
        };
        let mut queue = queue_entry.or_insert_with(AddressQueue::new);
        if let Some(old_tx) = queue.insert(nonce, queued_tx) {
            tracing::debug!(
                nonce,
                old_hash = ?old_tx.tx_hash,
                new_hash = ?tx_hash,
                "Replaced queued transaction with same nonce"
            );
        }
    }

    /// Remove a transaction by nonce
    pub fn evict(&self, credential_id: &CredentialId, nonce: u64) -> Option<QueuedTx<S, Rt>> {
        let mut entry = self.queues.get_mut(credential_id)?;
        let tx = entry.remove(nonce)?;

        // Clean up empty queue
        if entry.is_empty() {
            drop(entry);
            self.queues.remove(credential_id);
        }

        Some(tx)
    }

    /// Check if there's a contiguous sequence of transactions up to target_nonce
    pub fn has_prerequisites_to_nonce(
        &self,
        credential_id: &CredentialId,
        tx_nonce: u64,
        current_nonce: u64,
    ) -> bool {
        self.queues
            .get(credential_id)
            .map(|queue| queue.has_contiguous_sequence_to(tx_nonce, current_nonce))
            .unwrap_or(tx_nonce == current_nonce)
    }

    /// Drain all ready transactions starting from expected_nonce until a gap or error
    pub async fn drain_any_ready_transactions(
        &self,
        credential_id: CredentialId,
        mut expected_nonce: u64,
    ) {
        loop {
            // Lock and check if next tx is ready
            let queued_tx = {
                let mut queue = match self.queues.get_mut(&credential_id) {
                    Some(queue) => queue,
                    None => return, // No queue for this address
                };

                // Check if head has the expected nonce
                let tx = loop {
                    let head_entry = match queue.head() {
                        Some(entry) => entry,
                        None => return, // Empty queue
                    };
                    match head_entry.key().cmp(&expected_nonce) {
                        Ordering::Less => {
                            // Stale transaction in queue - evict and ignore.
                            head_entry.remove();
                            continue;
                        }
                        Ordering::Greater => {
                            // First transaction starts in the future - nothing ready to drain yet
                            return;
                        }
                        Ordering::Equal => {
                            // First transaction is the next expected nonce. Pop it and mark as executing.
                            let nonce = *head_entry.key();
                            let tx = head_entry.remove();
                            queue.mark_executing(nonce);
                            break tx;
                        }
                    }
                };

                tx
            };

            // Execute the transaction (outside lock)
            let result = self
                .submitter
                .execute_tx(
                    &queued_tx.tx,
                    queued_tx.tx_hash,
                    queued_tx.original_tx_queue_id,
                    "nonce_queue_trigger",
                )
                .await;

            // Check if we should continue draining
            let should_continue = result.as_ref().is_ok_and(|r| r.is_ok());

            // Send the raw result back to the original API handler task, so errors can be
            // correctly propagated to the user. If the receiver is no longer listening, ignore.
            let _ = queued_tx.result_sender.send(result);

            // After the waiting task has been notified, clean up the queue if it's empty
            if let Some(queue) = self.queues.get_mut(&credential_id) {
                if queue.is_empty() {
                    drop(queue);
                    self.queues.remove(&credential_id);
                }
            }

            if should_continue {
                expected_nonce += 1;
            } else {
                tracing::debug!("Transaction execution failed, stopping drain");
                return;
            }
        }
    }

    /// Handle nonce-based transaction: either execute immediately, queue for later, or reject.
    /// Returns the result from executing/queueing the transaction.
    ///
    /// The logic works as follows: if the transaction has a nonce a small (configurable) distance
    /// into the future, it's added to a queue (alongside a callback oneshot), and this function
    /// blocks on a loop waiting for either the oneshot result or a timeout.
    /// Whenever a transaction with a current nonce arrives, it's executed normally *and* the queue
    /// for that user is drained in a new background task: any contiguous range of transactions
    /// with valid nonces at the head of the queue are drained and executed. Every time a
    /// transaction is pulled from the queue it also forwards the results to the original oneshot,
    /// so the original `accept_tx()` task receives the data and can return it to the user.
    ///
    /// If the original task does not get a response across the oneshot before the timeout, the
    /// transaction is evicted from the queue, *unless* at that point there's already a queued
    /// transaction for every nonce (all the pre-reqs are satisifed). In that case the assumption
    /// is that it will get executed soon, provided all the txs are valid, so we don't evict. If
    /// any pre-req is rejected, the following txs will be evicted on the next timeout (assuming a
    /// new replacement pre-req with that nonce isn't submitted in the meantime of course).
    pub async fn handle_new_tx(
        &self,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        tx_nonce: u64,
        credential_id: CredentialId,
        original_tx_queue_id: u64,
    ) -> Result<
        Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>,
        SequencerStateUpdatorError,
    > {
        // First acquire lock on this user's queue, before checking the nonce. Otherwise there can
        // be a race condition if a new transaction were to execute after this tx checked its nonce
        // but before it added itself to the queue.
        let locked_user_queue = self.lock_for_address(credential_id);
        let current_nonce = self.submitter.get_current_nonce_for_user(&credential_id);
        let max_accepted_nonce = current_nonce + self.maximum_future_nonce_delta;

        match tx_nonce {
            nonce if nonce == current_nonce => {
                // Transaction has a valid nonce, execute immediately.
                drop(locked_user_queue); // Release the queue lock as we won't be using it
                let res = self
                    .submitter
                    .execute_tx(
                        &baked_tx,
                        tx_hash,
                        original_tx_queue_id,
                        "nonce_queue_immediate",
                    )
                    .await;

                // If the transaction succeeded, the user's nonce will have incremented.
                // Trigger a drain of the queue for any transactions which are now valid because of
                // this.
                if res.as_ref().is_ok_and(|r| r.is_ok()) {
                    let queues = self.clone();
                    let starting_nonce = current_nonce + 1; // Because we just submitted a transaction
                    tokio::spawn(async move {
                        queues
                            .drain_any_ready_transactions(credential_id, starting_nonce)
                            .await;
                    });
                }
                res
            }
            nonce if nonce > current_nonce && nonce < max_accepted_nonce => {
                // Transaction's nonce is in the future but within the queue threshold - enqueue it.
                let (result_sender, mut results_receiver) = oneshot::channel();
                Self::enqueue_and_unlock(
                    locked_user_queue,
                    result_sender,
                    baked_tx,
                    tx_hash,
                    tx_nonce,
                    original_tx_queue_id,
                );

                // Wait for either the transaction to be ready (prerequisites arrived) or timeout
                loop {
                    tokio::select! {
                        rx = &mut results_receiver => {
                            // The receiver contains the tx execution result.
                            break rx.unwrap_or_else(|_| {
                                // The oneshot sender was dropped. This should normally only happen
                                // on shutdown, but if a stale transaction somehow ends up in the
                                // queue it can be evicted (dropping it and the sender).
                                // Since the transaction was queued and therefore had an incorrect
                                // nonce to begin with, we conservatively reject with a nonce error.
                                let current_nonce = self.submitter.get_current_nonce_for_user(&credential_id);
                                err_invalid_nonce::<S, Rt>(
                                    tx_hash,
                                    tx_nonce,
                                    current_nonce,
                                    credential_id,
                                )
                            });
                        },
                        _ = tokio::time::sleep(std::time::Duration::from_millis(
                                self.future_nonce_transaction_timeout_millis
                        )) => {
                            // Timeout waiting for prerequisite transactions
                            let current_nonce = self.submitter.get_current_nonce_for_user(&credential_id);
                            if self.has_prerequisites_to_nonce(&credential_id, tx_nonce, current_nonce) {
                                // Still has a valid path to execution, keep waiting
                                continue;
                            } else {
                                // No path to execution, evict and reject
                                self.evict(&credential_id, tx_nonce);
                                break err_invalid_nonce::<S, Rt>(
                                    tx_hash,
                                    tx_nonce,
                                    current_nonce,
                                    credential_id,
                                );
                            }
                        }
                    }
                }
            }
            _ => {
                // Invalid nonce: either in the past or too far in the future
                err_invalid_nonce::<S, Rt>(tx_hash, tx_nonce, current_nonce, credential_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferred::{DoNewTxError, RollupBlockExecutorError};
    use sov_modules_api::{TxProcessingError, TransactionReceipt, SkippedTxContents};
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
            tx: FullyBakedTx {
                // Hacky fake data to help track nonces when TXs are sent through the queue
                data: vec![nonce].into(),
            },
            tx_hash: TxHash::from([0u8; 32]),
            original_tx_queue_id: 0,
            result_sender: sender,
        }
    }

    fn enqueue_mock_queued_tx(
        entry: dashmap::mapref::entry::Entry<CredentialId, AddressQueue<TestSpec, TestRuntime>>,
        nonce: u8,
    ) -> oneshot::Receiver<
        Result<TransactionReceiverResult<TestSpec, TestRuntime>, SequencerStateUpdatorError>
    > {
        let to_queue = create_mock_queued_tx(nonce);
        let (sender, receiver) = oneshot::channel();
        TxNonceQueues::<MockTxExecutionBackend, TestSpec, TestRuntime>::enqueue_and_unlock(
            entry,
            sender,
            to_queue.tx,
            to_queue.tx_hash,
            nonce.into(),
            to_queue.original_tx_queue_id,
        );
        receiver
    }

    fn nonce_from_queued_tx(tx: &QueuedTx<TestSpec, TestRuntime>) -> u8 {
        *tx.tx.data.first().unwrap()
    }

    #[test]
    fn test_has_contiguous_sequence_empty_queue() {
        let queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Empty queue should return false when target > current_nonce
        assert!(!queue.has_contiguous_sequence_to(5, 0));

        // But if target == current_nonce, return true even on empty queue (no prerequisites needed)
        assert!(queue.has_contiguous_sequence_to(0, 0));
        assert!(queue.has_contiguous_sequence_to(5, 5));
    }

    #[test]
    fn test_has_contiguous_sequence_target_equals_expected() {
        let queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // When target == expected_first, should return true immediately (edge case for eviction check)
        assert!(queue.has_contiguous_sequence_to(5, 5));
        assert!(queue.has_contiguous_sequence_to(0, 0));
        assert!(queue.has_contiguous_sequence_to(100, 100));
    }

    #[test]
    fn test_has_contiguous_sequence_wrong_start() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        queue.insert(3, create_mock_queued_tx(3));
        queue.insert(4, create_mock_queued_tx(4));
        queue.insert(5, create_mock_queued_tx(5));

        // Queue starts at 3, but we expect 0
        assert!(!queue.has_contiguous_sequence_to(5, 0));
        // Queue starts at 3, but we expect 1
        assert!(!queue.has_contiguous_sequence_to(5, 1));
        // Queue starts at 3, and we expect 3 - should work
        assert!(queue.has_contiguous_sequence_to(5, 3));
    }

    #[test]
    fn test_has_contiguous_sequence_with_gap() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        queue.insert(0, create_mock_queued_tx(0));
        queue.insert(1, create_mock_queued_tx(1));
        queue.insert(3, create_mock_queued_tx(3)); // Gap at 2
        queue.insert(4, create_mock_queued_tx(4));

        // Should succeed up to the gap
        assert!(queue.has_contiguous_sequence_to(0, 0));
        assert!(queue.has_contiguous_sequence_to(1, 0));
        assert!(queue.has_contiguous_sequence_to(2, 0));

        // Should fail when target is beyond the gap
        assert!(!queue.has_contiguous_sequence_to(3, 0));
        assert!(!queue.has_contiguous_sequence_to(4, 0));
    }

    #[test]
    fn test_has_contiguous_sequence_complete() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        queue.insert(5, create_mock_queued_tx(5));
        queue.insert(6, create_mock_queued_tx(6));
        queue.insert(7, create_mock_queued_tx(7));
        queue.insert(8, create_mock_queued_tx(8));

        // Should succeed for all nonces we have, and the next valid one
        assert!(queue.has_contiguous_sequence_to(7, 5));
        assert!(queue.has_contiguous_sequence_to(6, 5));
        assert!(queue.has_contiguous_sequence_to(5, 5));
        assert!(queue.has_contiguous_sequence_to(8, 5));
        assert!(queue.has_contiguous_sequence_to(9, 5));

        // Should fail when target is beyond what we have
        assert!(!queue.has_contiguous_sequence_to(10, 5));
    }

    #[test]
    fn test_has_contiguous_sequence_target_before_expected() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        queue.insert(5, create_mock_queued_tx(5));
        queue.insert(6, create_mock_queued_tx(6));

        // Target is before expected_first - should fail
        assert!(!queue.has_contiguous_sequence_to(4, 5));
    }

    #[test]
    fn test_has_contiguous_sequence_with_last_executed_at_start() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Queue has [1, 2, 3]
        queue.insert(1, create_mock_queued_tx(1));
        queue.insert(2, create_mock_queued_tx(2));
        queue.insert(3, create_mock_queued_tx(3));

        // Should fail, since we don't have tx 0
        assert!(!queue.has_contiguous_sequence_to(4, 0));
        assert!(!queue.has_contiguous_sequence_to(3, 0));
        assert!(!queue.has_contiguous_sequence_to(2, 0));
        assert!(!queue.has_contiguous_sequence_to(1, 0));

        // Now we mark tx 0 as executing, even though the user's nonce isn't updated yet
        queue.mark_executing(0);

        // Should succeed - last_executed fills the first position
        assert!(queue.has_contiguous_sequence_to(4, 0));
        assert!(queue.has_contiguous_sequence_to(3, 0));
        assert!(queue.has_contiguous_sequence_to(2, 0));
        assert!(queue.has_contiguous_sequence_to(1, 0));
        assert!(queue.has_contiguous_sequence_to(0, 0));

        // Should fail - we don't have nonce 4
        assert!(!queue.has_contiguous_sequence_to(5, 0));
    }

    #[test]
    fn test_has_contiguous_sequence_with_last_executed_alone() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Empty queue, but last_executed = 5
        queue.mark_executing(5);

        // Should succeed when target equals last_executed
        assert!(queue.has_contiguous_sequence_to(5, 5));
        // And for the next valid nonce, too, despite the queue being empty
        assert!(queue.has_contiguous_sequence_to(6, 5));

        // Should fail when nonce has a gap
        assert!(!queue.has_contiguous_sequence_to(7, 5));
        // Should fail in the past
        assert!(!queue.has_contiguous_sequence_to(4, 5));
    }

    #[test]
    fn test_has_contiguous_sequence_with_last_executed_stale() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Queue has [2, 3], last_executed = 0 (stale, before current_nonce)
        queue.insert(2, create_mock_queued_tx(2));
        queue.insert(3, create_mock_queued_tx(3));
        queue.mark_executing(0);

        // Should succeed - stale marker doesn't affect check starting at 2
        assert!(queue.has_contiguous_sequence_to(3, 2));

        // Should fail - there's a gap at 1
        assert!(!queue.has_contiguous_sequence_to(3, 1));
    }

    #[test]
    fn test_has_contiguous_sequence_with_last_executed_nonce_is_stale() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Queue has [5, 6], last_executed = 4
        queue.insert(5, create_mock_queued_tx(2));
        queue.insert(6, create_mock_queued_tx(3));
        queue.mark_executing(4);

        // Nonce is stale at 1 but we know 4 is already executing, so should succeed
        assert!(queue.has_contiguous_sequence_to(4, 1));
        assert!(queue.has_contiguous_sequence_to(5, 1));
        assert!(queue.has_contiguous_sequence_to(6, 1));
        assert!(queue.has_contiguous_sequence_to(7, 1));
        assert!(!queue.has_contiguous_sequence_to(8, 1)); // Lacks nonce 7 as a prerequisite

        // 0 is in the past compared to the user's current nonce
        assert!(!queue.has_contiguous_sequence_to(0, 1));
        // 1 matches the user's current nonce
        assert!(queue.has_contiguous_sequence_to(1, 1));
        // 2 and 3 are between the current nonce and the last_executed, so we keep them in the
        // queue for now
        assert!(queue.has_contiguous_sequence_to(2, 1));
        assert!(queue.has_contiguous_sequence_to(3, 1));
    }

    #[test]
    fn test_has_contiguous_sequence_with_gap_after_last_executed() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Queue has [2], last_executed = 0 (with gap)
        queue.insert(2, create_mock_queued_tx(2));
        queue.mark_executing(0);

        // Since we're currently executing 0, then nonce 1 is valid
        assert!(queue.has_contiguous_sequence_to(1, 0));
        // But since there's a gap, nonce 2 is not contiguous
        assert!(!queue.has_contiguous_sequence_to(2, 0));
    }

    #[test]
    fn test_address_queue_insert_and_remove() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Insert some transactions
        assert!(queue.insert(1, create_mock_queued_tx(1)).is_none());
        assert!(queue.insert(2, create_mock_queued_tx(2)).is_none());
        assert!(queue.insert(3, create_mock_queued_tx(3)).is_none());

        // Replace existing transaction
        let old_tx = queue.insert(2, create_mock_queued_tx(2));
        assert!(old_tx.is_some());
        assert_eq!(nonce_from_queued_tx(&old_tx.unwrap()), 2);

        // Remove transactions
        assert!(queue.remove(1).is_some());
        assert!(queue.remove(1).is_none()); // Already removed
        assert!(queue.remove(2).is_some());
        assert!(queue.remove(3).is_some());

        assert!(queue.is_empty());
    }

    #[test]
    fn test_address_queue_head() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Empty queue has no head
        assert!(queue.head().is_none());

        // Insert out of order
        queue.insert(5, create_mock_queued_tx(5));
        queue.insert(3, create_mock_queued_tx(3));
        queue.insert(7, create_mock_queued_tx(7));

        // Head should be the lowest nonce (BTreeMap ordering)
        let head = queue.head();
        assert!(head.is_some());
        let head = head.unwrap();
        assert_eq!(*head.key(), 3);

        // Remove head and check next
        head.remove();
        let next_head = queue.head();
        assert_eq!(*next_head.unwrap().key(), 5);
    }

    #[test]
    fn test_tx_nonce_queues_basic_operations() {
        let (queues, _) = default_test_tx_nonce_queues();

        let credential_id = CredentialId::from([1u8; 32]);

        // Enqueue some transactions
        let entry = queues.lock_for_address(credential_id);
        enqueue_mock_queued_tx(entry, 5);
        let entry = queues.lock_for_address(credential_id);
        enqueue_mock_queued_tx(entry, 6);

        // Check prerequisites
        assert!(queues.has_prerequisites_to_nonce(&credential_id, 6, 5));
        assert!(queues.has_prerequisites_to_nonce(&credential_id, 7, 5)); // Next valid one
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 6, 4)); // Wrong start
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 8, 5)); // Beyond what we have

        // Remove a transaction
        let removed = queues.evict(&credential_id, 5);
        assert!(removed.is_some());
        assert_eq!(nonce_from_queued_tx(&removed.unwrap()), 5);

        // Check prerequisites again
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 6, 5)); // Gap now

        // Remove non-existent
        assert!(queues.evict(&credential_id, 5).is_none());

        // Remove last transaction - queue should be cleaned up
        assert!(queues.evict(&credential_id, 6).is_some());
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 6, 5));
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

    #[tokio::test]
    async fn test_drain_evicts_stale_and_executes_ready() {
        let (queues, backend) = default_test_tx_nonce_queues();
        let credential_id = CredentialId::from([1u8; 32]);

        // Enqueue transactions with nonces 3, 4, 5
        for nonce in [3, 4, 5] {
            let entry = queues.lock_for_address(credential_id);
            enqueue_mock_queued_tx(entry, nonce);
        }

        // Drain starting from nonce 5 (3 and 4 should be silently evicted, 5 should execute)
        queues.drain_any_ready_transactions(credential_id, 5).await;

        let executed_nonces = backend.get_executed_nonces();
        assert_eq!(
            executed_nonces,
            vec![5],
            "Should execute only nonce 5, not stale nonces 3 and 4"
        );

        // Verify queue is empty (and therefore pruned)
        assert!(queues.queues.get(&credential_id).is_none());
    }

    #[tokio::test]
    async fn test_drain_stops_at_gap() {
        let (queues, backend) = default_test_tx_nonce_queues();
        let credential_id = CredentialId::from([1u8; 32]);

        // Enqueue transactions: 0, 1, 3, 4 (gap at 2)
        for nonce in [0, 1, 3, 4] {
            let entry = queues.lock_for_address(credential_id);
            enqueue_mock_queued_tx(entry, nonce);
        }

        // Drain starting from nonce 0
        queues.drain_any_ready_transactions(credential_id, 0).await;

        // Should have executed exactly 2 transactions (0 and 1), then stopped at gap
        let executed_nonces = backend.get_executed_nonces();
        assert_eq!(
            executed_nonces,
            vec![0, 1],
            "Should execute nonces 0 and 1, then stop at gap (missing nonce 2)"
        );

        // Transactions 3 and 4 should still be in queue
        assert!(queues.has_prerequisites_to_nonce(&credential_id, 4, 3));
    }

    /// Mock implementation of TxExecutionBackend for testing handle_new_tx
    #[derive(Clone)]
    struct MockTxExecutionBackend {
        current_nonce: Arc<AtomicU64>,
        executed_txs: Arc<Mutex<Vec<(TxHash, u64)>>>,
        should_fail_nonce: Option<u64>,
        execution_delay: Duration,
    }

    #[allow(dead_code)]
    impl MockTxExecutionBackend {
        fn new() -> Self {
            Self {
                current_nonce: Arc::new(AtomicU64::new(0)),
                executed_txs: Arc::new(Mutex::new(Vec::new())),
                should_fail_nonce: None,
                execution_delay: Duration::from_millis(0),
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

        fn with_failure_at_nonce(mut self, nonce: u64) -> Self {
            self.should_fail_nonce = Some(nonce);
            self
        }

        fn get_executed_nonces(&self) -> Vec<u64> {
            self.executed_txs.lock().unwrap().iter().map(|(_, n)| *n).collect()
        }

        fn increment_nonce(&self) {
            self.current_nonce.fetch_add(1, Ordering::SeqCst);
        }
    }

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
        ) -> Result<
            Result<oneshot::Receiver<AcceptedTx<Confirmation<TestSpec, TestRuntime>>>, AcceptTxError<TestSpec>>,
            SequencerStateUpdatorError,
        > {
            // Add delay if configured
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

            // Simulate successful execution
            let (tx, rx) = oneshot::channel();
            let baked_tx_clone = baked_tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(AcceptedTx::<Confirmation<TestSpec, TestRuntime>> {
                    tx: baked_tx_clone,
                    tx_hash,
                    confirmation: create_default_confirmation(),
                });
            });

            // Auto-increment nonce on successful execution
            self.increment_nonce();

            Ok(Ok(rx))
        }
    }

    fn create_test_tx(nonce: u8) -> FullyBakedTx {
        FullyBakedTx {
            // Use nonce as data to track it
            data: vec![nonce].into(),
        }
    }

    /// Helper to create a default TxNonceQueues for tests that don't care about specific config
    fn default_test_tx_nonce_queues() -> (TxNonceQueues<MockTxExecutionBackend, TestSpec, TestRuntime>, MockTxExecutionBackend) {
        let backend = MockTxExecutionBackend::new();
        let queues = TxNonceQueues::new(
            backend.clone(),
            100, // generous max_future_nonce_delta
            60000, // 60 second timeout (won't trigger in normal tests)
        );
        (queues, backend)
    }

    #[tokio::test]
    async fn test_immediate_execution_triggers_drain() {
        let backend = MockTxExecutionBackend::new().with_current_nonce(0);
        let queues = TxNonceQueues::new(
            backend.clone(),
            10, // max_future_nonce_delta
            5000, // timeout (long enough to not trigger)
        );

        let credential_id = CredentialId::from([1u8; 32]);

        // Submit TXs with nonces 1, 2, 3 (should be queued)
        let mut handles = vec![];
        for nonce in 1u8..=3 {
            let queues = queues.clone();
            let handle = tokio::spawn(async move {
                queues
                    .handle_new_tx(
                        create_test_tx(nonce),
                        TxHash::from([nonce; 32]),
                        nonce as u64,
                        credential_id,
                        0,
                    )
                    .await
            });
            handles.push(handle);
        }

        // Give queued TXs time to settle
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Submit TX with nonce 0 (current nonce) - should execute immediately and trigger drain
        let result = queues
            .handle_new_tx(
                create_test_tx(0),
                TxHash::from([0; 32]),
                0,
                credential_id,
                0,
            )
            .await;

        // TX 0 should execute successfully
        assert!(result.is_ok());
        let inner = result.unwrap();
        assert!(inner.is_ok());
        let rx = inner.unwrap();
        assert!(rx.await.is_ok());

        // Wait for all queued TXs to complete
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok());
            let inner = result.unwrap();
            assert!(inner.is_ok());
            let rx = inner.unwrap();
            assert!(rx.await.is_ok());
        }

        // Verify all 4 TXs executed in order
        let executed = backend.get_executed_nonces();
        assert_eq!(
            executed,
            vec![0, 1, 2, 3],
            "All transactions should execute in order after filling the gap"
        );

        // Verify current nonce advanced
        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 4);
    }

    #[tokio::test]
    async fn test_reject_past_nonce() {
        let backend = MockTxExecutionBackend::new().with_current_nonce(5);
        let queues = TxNonceQueues::new(backend.clone(), 10, 5000);

        let credential_id = CredentialId::from([1u8; 32]);

        // Submit TX with nonce 4 (past nonce, current is 5)
        let result = queues
            .handle_new_tx(
                create_test_tx(4),
                TxHash::from([4; 32]),
                4,
                credential_id,
                0,
            )
            .await;

        // Should get an error result
        assert!(result.is_ok());
        let inner = result.unwrap();
        assert!(inner.is_err());

        match inner.unwrap_err() {
            AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
            )) => {
                // Verify it's a nonce error
                match receipt.receipt {
                    sov_rollup_interface::stf::TxEffect::Skipped(contents) => {
                        match contents.error {
                            TxProcessingError::CheckUniquenessFailed(msg) => {
                                assert!(msg.contains("bad nonce"), "Error should mention bad nonce: {}", msg);
                                assert!(msg.contains("expected: 5"), "Error should mention expected nonce 5: {}", msg);
                                assert!(msg.contains("found: 4"), "Error should mention found nonce 4: {}", msg);
                            }
                            _ => panic!("Expected CheckUniquenessFailed error"),
                        }
                    }
                    _ => panic!("Expected Skipped receipt"),
                }
            }
            _ => panic!("Expected UnsuccessfulTransaction error"),
        }

        // Verify no TXs were executed
        assert!(backend.get_executed_nonces().is_empty());

        // Verify nonce didn't change
        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 5);
    }

    #[tokio::test]
    async fn test_reject_too_far_future_nonce() {
        let backend = MockTxExecutionBackend::new().with_current_nonce(0);
        let queues = TxNonceQueues::new(
            backend.clone(),
            10, // max_future_nonce_delta = 10, so max valid is 0+10 = 10
            5000,
        );

        let credential_id = CredentialId::from([1u8; 32]);

        // Submit TX with nonce 11 (too far in future, max valid is 10)
        let result = queues
            .handle_new_tx(
                create_test_tx(11),
                TxHash::from([11; 32]),
                11,
                credential_id,
                0,
            )
            .await;

        // Should get an error result
        assert!(result.is_ok());
        let inner = result.unwrap();
        assert!(inner.is_err());

        match inner.unwrap_err() {
            AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
            )) => {
                // Verify it's a nonce error
                match receipt.receipt {
                    sov_rollup_interface::stf::TxEffect::Skipped(contents) => {
                        match contents.error {
                            TxProcessingError::CheckUniquenessFailed(msg) => {
                                assert!(msg.contains("bad nonce"), "Error should mention bad nonce: {}", msg);
                            }
                            _ => panic!("Expected CheckUniquenessFailed error"),
                        }
                    }
                    _ => panic!("Expected Skipped receipt"),
                }
            }
            _ => panic!("Expected UnsuccessfulTransaction error"),
        }

        // Verify no TXs were executed
        assert!(backend.get_executed_nonces().is_empty());

        // Verify nonce didn't change
        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 0);
    }

    #[tokio::test]
    async fn test_queue_future_nonce_then_fill_gap() {
        let backend = MockTxExecutionBackend::new().with_current_nonce(0);
        let queues = TxNonceQueues::new(backend.clone(), 10, 5000);
        let credential_id = CredentialId::from([1u8; 32]);

        // Submit TX with nonce 2 (should queue, current is 0)
        let handle_2 = tokio::spawn({
            let queues = queues.clone();
            async move {
                queues
                    .handle_new_tx(
                        create_test_tx(2),
                        TxHash::from([2; 32]),
                        2,
                        credential_id,
                        0,
                    )
                    .await
            }
        });

        // Give it time to queue
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Submit TX with nonce 0 (should execute immediately, but NOT drain N+2 due to gap at N+1)
        let result_0 = queues
            .handle_new_tx(
                create_test_tx(0),
                TxHash::from([0; 32]),
                0,
                credential_id,
                0,
            )
            .await;
        assert!(result_0.is_ok());
        let rx_0 = result_0.unwrap().unwrap();
        assert!(rx_0.await.is_ok());

        // Give drain task time to run (it shouldn't drain N+2)
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify only N=0 executed, N+2 still queued
        assert_eq!(backend.get_executed_nonces(), vec![0]);
        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 1);

        // Now submit TX with nonce 1 (should execute and trigger drain of N+2)
        let result_1 = queues
            .handle_new_tx(
                create_test_tx(1),
                TxHash::from([1; 32]),
                1,
                credential_id,
                0,
            )
            .await;
        assert!(result_1.is_ok());
        let rx_1 = result_1.unwrap().unwrap();
        assert!(rx_1.await.is_ok());

        // Wait for N+2 to complete
        let result_2 = handle_2.await.unwrap();
        assert!(result_2.is_ok());
        let rx_2 = result_2.unwrap().unwrap();
        assert!(rx_2.await.is_ok());

        // Verify all executed in order
        assert_eq!(backend.get_executed_nonces(), vec![0, 1, 2]);
        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 3);
    }

    #[tokio::test]
    async fn test_queued_tx_timeout_without_prerequisites() {
        let backend = MockTxExecutionBackend::new().with_current_nonce(0);
        let queues = TxNonceQueues::new(
            backend.clone(),
            10,
            200, // Short timeout (200ms) to make test fast
        );
        let credential_id = CredentialId::from([1u8; 32]);

        // Submit TX with nonce 1 (will queue and wait)
        let handle = tokio::spawn({
            let queues = queues.clone();
            async move {
                queues
                    .handle_new_tx(
                        create_test_tx(1),
                        TxHash::from([1; 32]),
                        1,
                        credential_id,
                        0,
                    )
                    .await
            }
        });

        // Wait for timeout to trigger (200ms timeout + some buffer)
        tokio::time::sleep(Duration::from_millis(300)).await;

        // TX should have been evicted with nonce error
        let result = handle.await.unwrap();
        assert!(result.is_ok());
        let inner = result.unwrap();
        assert!(inner.is_err());

        // Verify it's a nonce error
        match inner.unwrap_err() {
            AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
            )) => {
                match receipt.receipt {
                    sov_rollup_interface::stf::TxEffect::Skipped(contents) => {
                        match contents.error {
                            TxProcessingError::CheckUniquenessFailed(msg) => {
                                assert!(msg.contains("bad nonce"), "Error should mention bad nonce: {}", msg);
                                assert!(msg.contains("expected: 0"), "Error should mention expected nonce 0: {}", msg);
                                assert!(msg.contains("found: 1"), "Error should mention found nonce 1: {}", msg);
                            }
                            _ => panic!("Expected CheckUniquenessFailed error"),
                        }
                    }
                    _ => panic!("Expected Skipped receipt"),
                }
            }
            _ => panic!("Expected UnsuccessfulTransaction error"),
        }

        // Verify no TXs were executed
        assert!(backend.get_executed_nonces().is_empty());
        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 0);

        // Verify TX was removed from queue
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 1, 0));
    }

    #[tokio::test]
    async fn test_queued_tx_timeout_loops_if_all_prerequisites_present() {
        let backend = MockTxExecutionBackend::new()
            .with_current_nonce(0)
            .with_execution_delay(Duration::from_millis(200)); // Each TX takes 200ms

        let queues = TxNonceQueues::new(
            backend.clone(),
            10,
            500, // Timeout is 250ms, longer than execution delay but shorter than total time to
                 // execute all queued TXs
        );
        let credential_id = CredentialId::from([1u8; 32]);

        // Submit TXs with nonces 1-5 (all will queue)
        let mut handles = vec![];
        for nonce in 1u8..=5 {
            let queues = queues.clone();
            let handle = tokio::spawn(async move {
                queues
                    .handle_new_tx(
                        create_test_tx(nonce),
                        TxHash::from([nonce; 32]),
                        nonce as u64,
                        credential_id,
                        0,
                    )
                    .await
            });
            handles.push(handle);
        }

        // Give TXs time to queue
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Now submit TX with nonce 0 to trigger drain
        let result_0 = queues
            .handle_new_tx(
                create_test_tx(0),
                TxHash::from([0; 32]),
                0,
                credential_id,
                0,
            )
            .await;
        assert!(result_0.is_ok());
        let rx_0 = result_0.unwrap().unwrap();
        assert!(rx_0.await.is_ok());

        // Wait for all queued TXs to complete
        // Each TX takes 200ms, so 5 TXs = ~1000ms total
        // Multiple timeouts (250ms each) will fire during this time
        // but TXs should NOT be evicted because they have prerequisites
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok(), "TX should not timeout - has prerequisites");
            let rx = result.unwrap().unwrap();
            assert!(rx.await.is_ok());
        }

        // Verify all TXs executed in order
        assert_eq!(
            backend.get_executed_nonces(),
            vec![0, 1, 2, 3, 4, 5],
            "All transactions should execute despite timeouts firing during drain"
        );
        assert_eq!(backend.get_current_nonce_for_user(&credential_id), 6);
    }

    #[tokio::test]
    async fn test_oneshot_sender_dropped_returns_error() {
        let backend = MockTxExecutionBackend::new().with_current_nonce(0);
        let queues = TxNonceQueues::new(backend.clone(), 10, 500);
        let credential_id = CredentialId::from([1u8; 32]);

        // Submit TX with nonce 1 (will queue)
        let handle = tokio::spawn({
            let queues = queues.clone();
            async move {
                queues
                    .handle_new_tx(
                        create_test_tx(1),
                        TxHash::from([1; 32]),
                        1,
                        credential_id,
                        0,
                    )
                    .await
            }
        });

        // Give it time to queue
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Manually evict the TX (drops the oneshot sender)
        let evicted = queues.evict(&credential_id, 1);
        assert!(evicted.is_some(), "TX should have been queued");

        // Add a timeout to detect if the task hangs
        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("Task should complete within 2 seconds (sender was dropped)")
            .unwrap();

        // When the sender is dropped, the receiver errors, which gets mapped to a nonce error
        assert!(result.is_ok(), "Should not have sequencer error");
        let inner = result.unwrap();

        // The inner result should be an error (AcceptTxError with nonce error)
        assert!(inner.is_err(), "Should have nonce error when sender dropped");

        // Verify it's a nonce error
        match inner.unwrap_err() {
            AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                RollupBlockExecutorError::UnsuccessfulTransaction { receipt },
            )) => {
                match receipt.receipt {
                    sov_rollup_interface::stf::TxEffect::Skipped(contents) => {
                        match contents.error {
                            TxProcessingError::CheckUniquenessFailed(msg) => {
                                assert!(msg.contains("bad nonce"), "Error should mention bad nonce: {}", msg);
                            }
                            _ => panic!("Expected CheckUniquenessFailed error, got: {:?}", contents.error),
                        }
                    }
                    _ => panic!("Expected Skipped receipt"),
                }
            }
            other => panic!("Expected nonce error, got: {:?}", other),
        }

        // Verify no TXs were executed
        assert!(backend.get_executed_nonces().is_empty());
    }
}
