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

pub struct QueuedTx<S: Spec, Rt: Runtime<S>> {
    pub tx: FullyBakedTx,
    pub tx_hash: TxHash,
    pub nonce: u64,
    pub original_tx_queue_id: u64,
    pub result_sender: oneshot::Sender<
        Result<
            Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>,
            SequencerStateUpdatorError,
        >,
    >,
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
    pub fn enqueue_with_lock(
        queue_entry: dashmap::mapref::entry::Entry<CredentialId, AddressQueue<S, Rt>>,
        tx: QueuedTx<S, Rt>,
    ) {
        let nonce = tx.nonce;
        let new_hash = tx.tx_hash;
        let mut queue = queue_entry.or_insert_with(AddressQueue::new);
        if let Some(old_tx) = queue.insert(nonce, tx) {
            tracing::debug!(
                nonce,
                old_hash = ?old_tx.tx_hash,
                new_hash = ?new_hash,
                "Replaced queued transaction with same nonce"
            );
        }
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
                let head_entry = match queue.head() {
                    Some(entry) => entry,
                    None => return, // Empty queue
                };

                if *head_entry.key() != expected_nonce {
                    return; // Not ready yet
                }

                // Pop the transaction
                let tx = head_entry.remove();

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
            // correctly propagated to the user
            if queued_tx.result_sender.send(result).is_err() {
                tracing::debug!("Waiting task dropped receiver, stopping drain");
                return;
            }

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
    use super::*;
    use sov_test_utils::runtime::TestOptimisticRuntime;
    use sov_test_utils::TestSpec;
    use tokio::sync::oneshot;

    type TestRuntime = TestOptimisticRuntime<TestSpec>;

    // Helper to create a mock QueuedTx for testing
    fn create_mock_queued_tx(nonce: u64) -> QueuedTx<TestSpec, TestRuntime> {
        let (sender, _receiver) = oneshot::channel();
        QueuedTx {
            tx: FullyBakedTx {
                data: vec![].into(),
            },
            tx_hash: TxHash::from([0u8; 32]),
            nonce,
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
        let entry = queues.lock_for_address(credential_id.clone());
        TxNonceQueues::enqueue_with_lock(entry, create_mock_queued_tx(5));

        let entry = queues.lock_for_address(credential_id.clone());
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
}
