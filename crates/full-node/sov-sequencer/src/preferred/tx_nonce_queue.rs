use dashmap::DashMap;
use sov_modules_api::{FullyBakedTx, Runtime, Spec};
use sov_rollup_interface::{crypto::CredentialId, TxHash};
use std::cmp::Ordering;
use std::collections::{btree_map::OccupiedEntry, BTreeMap};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::common::AcceptedTx;

use super::sync_sequencer_state::{
    AcceptTxError, SequencerStateUpdator, SequencerStateUpdatorError,
};
use super::Confirmation;

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
}

impl<S: Spec, Rt: Runtime<S>> AddressQueue<S, Rt> {
    fn new() -> Self {
        Self {
            txs: BTreeMap::new(),
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

    fn has_contiguous_sequence_to(&self, tx_nonce: u64, next_nonce: u64) -> bool {
        match tx_nonce.cmp(&next_nonce) {
            Ordering::Less => false, // tx is in the past
            Ordering::Equal => true, // tx is ready now - no prerequisites necessary
            Ordering::Greater => {
                let mut expected = next_nonce;
                for &nonce in self.txs.keys() {
                    if nonce > tx_nonce {
                        break;
                    }
                    if nonce != expected {
                        return false;
                    }
                    expected += 1;
                }
                expected > tx_nonce
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }
}

#[derive(Default)]
pub struct TxNonceQueues<S: Spec, Rt: Runtime<S>> {
    queues: DashMap<CredentialId, AddressQueue<S, Rt>>,
}

#[allow(dead_code)]
impl<S: Spec, Rt: Runtime<S>> TxNonceQueues<S, Rt> {
    pub fn new() -> Self {
        Self {
            queues: DashMap::new(),
        }
    }

    /// Lock the queue for a specific address for atomic nonce check + enqueue
    pub fn lock_for_address(
        &self,
        credential_id: CredentialId,
    ) -> dashmap::mapref::entry::Entry<CredentialId, AddressQueue<S, Rt>> {
        self.queues.entry(credential_id)
    }

    /// Enqueue a transaction (caller should hold lock from lock_for_address)
    pub fn enqueue_from_lock(
        queue_entry: dashmap::mapref::entry::Entry<CredentialId, AddressQueue<S, Rt>>,
        tx: FullyBakedTx,
        tx_hash: TxHash,
        nonce: u64,
        original_tx_queue_id: u64,
    ) -> oneshot::Receiver<Result<TransactionReceiverResult<S, Rt>, SequencerStateUpdatorError>> {
        let (result_sender, result_receiver) = oneshot::channel();
        let queued_tx = QueuedTx {
            tx,
            tx_hash,
            original_tx_queue_id,
            result_sender
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

        result_receiver
    }

    /// Remove a transaction by nonce
    pub fn remove(&self, credential_id: &CredentialId, nonce: u64) -> Option<QueuedTx<S, Rt>> {
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
        target_nonce: u64,
        expected_first: u64,
    ) -> bool {
        self.queues
            .get(credential_id)
            .map(|queue| queue.has_contiguous_sequence_to(target_nonce, expected_first))
            .unwrap_or(false)
    }

    /// Drain all ready transactions starting from expected_nonce until a gap or error
    pub async fn drain_any_ready_transactions(
        &self,
        credential_id: CredentialId,
        mut expected_nonce: u64,
        updator: Arc<SequencerStateUpdator<S, Rt>>,
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
                            // First transaction is the next expected nonce. Pop it and return it.
                            break head_entry.remove();
                        }
                    }
                };

                // Clean up empty queue
                if queue.is_empty() {
                    drop(queue);
                    self.queues.remove(&credential_id);
                }

                tx
            };

            // Execute the transaction (outside lock)
            let result = updator
                .accept_tx_msg(
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

            if should_continue {
                expected_nonce += 1;
            } else {
                tracing::debug!("Transaction execution failed, stopping drain");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::sync_sequencer_state::Message;
    use super::super::sync_sequencer_state::SequencerStateUpdator;
    use super::*;
    use sov_modules_api::SkippedTxContents;
    use sov_modules_api::TxProcessingError;
    use sov_test_utils::runtime::TestOptimisticRuntime;
    use sov_test_utils::TestSpec;
    use std::sync::atomic::AtomicU32;
    use tokio::sync::mpsc;
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
            nonce: nonce.into(),
            original_tx_queue_id: 0,
            result_sender: sender,
        }
    }

    #[test]
    fn test_has_contiguous_sequence_empty_queue() {
        let queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Empty queue should return false when target > next_nonce
        assert!(!queue.has_contiguous_sequence_to(5, 0));

        // But if target == next_nonce, return true even on empty queue (no prerequisites needed)
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
        assert!(queue.has_contiguous_sequence_to(1, 0));
        assert!(queue.has_contiguous_sequence_to(0, 0));

        // Should fail when target is beyond the gap
        assert!(!queue.has_contiguous_sequence_to(2, 0));
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

        // Should succeed for all nonces we have
        assert!(queue.has_contiguous_sequence_to(7, 5));
        assert!(queue.has_contiguous_sequence_to(6, 5));
        assert!(queue.has_contiguous_sequence_to(5, 5));
        assert!(queue.has_contiguous_sequence_to(8, 5));

        // Should fail when target is beyond what we have
        assert!(!queue.has_contiguous_sequence_to(9, 5));
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
    fn test_address_queue_insert_and_remove() {
        let mut queue: AddressQueue<TestSpec, TestRuntime> = AddressQueue::new();

        // Insert some transactions
        assert!(queue.insert(1, create_mock_queued_tx(1)).is_none());
        assert!(queue.insert(2, create_mock_queued_tx(2)).is_none());
        assert!(queue.insert(3, create_mock_queued_tx(3)).is_none());

        // Replace existing transaction
        let old_tx = queue.insert(2, create_mock_queued_tx(2));
        assert!(old_tx.is_some());
        assert_eq!(old_tx.unwrap().nonce, 2);

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
        let queues: TxNonceQueues<TestSpec, TestRuntime> = TxNonceQueues::new();

        let credential_id = CredentialId::from([1u8; 32]);

        // Enqueue some transactions
        let entry = queues.lock_for_address(credential_id);
        TxNonceQueues::enqueue_with_lock(entry, create_mock_queued_tx(5));

        let entry = queues.lock_for_address(credential_id);
        TxNonceQueues::enqueue_with_lock(entry, create_mock_queued_tx(6));

        // Check prerequisites
        assert!(queues.has_prerequisites_to_nonce(&credential_id, 6, 5));
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 6, 4)); // Wrong start
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 7, 5)); // Beyond what we have

        // Remove a transaction
        let removed = queues.remove(&credential_id, 5);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().nonce, 5);

        // Check prerequisites again
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 6, 5)); // Gap now

        // Remove non-existent
        assert!(queues.remove(&credential_id, 5).is_none());

        // Remove last transaction - queue should be cleaned up
        assert!(queues.remove(&credential_id, 6).is_some());
        assert!(!queues.has_prerequisites_to_nonce(&credential_id, 6, 5));
    }

    // Helper to create mock updator for tests
    #[allow(clippy::type_complexity)]
    fn create_mock_updator() -> (
        Arc<SequencerStateUpdator<TestSpec, TestRuntime>>,
        tokio::sync::mpsc::Receiver<
            super::super::sync_sequencer_state::Message<TestSpec, TestRuntime>,
        >,
    ) {
        let (msg_tx, msg_rx) = tokio::sync::mpsc::channel(10);
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
        let updator = Arc::new(SequencerStateUpdator {
            channel_size: Arc::new(AtomicU32::new(0)),
            message_sender: msg_tx,
            shutdown_receiver: shutdown_rx,
        });
        (updator, msg_rx)
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

    async fn mock_sequencer_state(
        mut msg_rx: mpsc::Receiver<Message<TestSpec, TestRuntime>>,
    ) -> Vec<u8> {
        let mut executed_nonces = Vec::new();
        while let Some(msg) = msg_rx.recv().await {
            match msg {
                Message::AcceptTx { resp, baked_tx, .. } => {
                    let (confirm_tx, confirm_rx) = oneshot::channel();
                    // We set the data to just be the one-byte nonce
                    executed_nonces.push(*baked_tx.data.first().unwrap());
                    tokio::spawn(async move {
                        let _ =
                            confirm_tx.send(AcceptedTx::<Confirmation<TestSpec, TestRuntime>> {
                                tx: baked_tx,
                                tx_hash: TxHash::from([0u8; 32]),
                                confirmation: create_default_confirmation(),
                            });
                    });
                    let _ = resp.send(Ok(confirm_rx));
                }
                _ => panic!("Unexpected message type"),
            }
        }
        executed_nonces
    }

    #[tokio::test]
    async fn test_drain_evicts_stale_and_executes_ready() {
        let queues: TxNonceQueues<TestSpec, TestRuntime> = TxNonceQueues::new();
        let credential_id = CredentialId::from([1u8; 32]);

        let (updator, msg_rx) = create_mock_updator();

        // Enqueue transactions with nonces 3, 4, 5
        for nonce in [3, 4, 5] {
            let entry = queues.lock_for_address(credential_id);
            TxNonceQueues::enqueue_with_lock(entry, create_mock_queued_tx(nonce));
        }

        // Spawn mock message handler that tracks which nonces are executed
        let handler = tokio::spawn(async move { mock_sequencer_state(msg_rx).await });

        // Drain starting from nonce 5 (3 and 4 should be silently evicted, 5 should execute)
        queues
            .drain_any_ready_transactions(credential_id, 5, updator.clone())
            .await;

        // Close channels so handler task ends
        drop(updator);

        let executed_nonces = handler.await.unwrap();
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
        let queues: TxNonceQueues<TestSpec, TestRuntime> = TxNonceQueues::new();
        let credential_id = CredentialId::from([1u8; 32]);

        let (updator, msg_rx) = create_mock_updator();

        // Enqueue transactions: 0, 1, 3, 4 (gap at 2)
        for nonce in [0, 1, 3, 4] {
            let entry = queues.lock_for_address(credential_id);
            TxNonceQueues::enqueue_with_lock(entry, create_mock_queued_tx(nonce));
        }

        // Spawn mock message handler
        let handler = tokio::spawn(async move { mock_sequencer_state(msg_rx).await });

        // Drain starting from nonce 0
        queues
            .drain_any_ready_transactions(credential_id, 0, updator.clone())
            .await;

        drop(updator);

        // Should have executed exactly 2 transactions (0 and 1), then stopped at gap
        let executed_nonces = handler.await.unwrap();
        assert_eq!(
            executed_nonces,
            vec![0, 1],
            "Should execute nonces 0 and 1, then stop at gap (missing nonce 2)"
        );

        // Transactions 3 and 4 should still be in queue
        assert!(queues.has_prerequisites_to_nonce(&credential_id, 4, 3));
    }
}
