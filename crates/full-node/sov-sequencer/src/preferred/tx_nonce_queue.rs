use dashmap::DashMap;
use sov_modules_api::{FullyBakedTx, Runtime, Spec};
use sov_rollup_interface::{crypto::CredentialId, TxHash};
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

    fn has_contiguous_sequence_to(&self, target_nonce: u64, expected_first: u64) -> bool {
        if target_nonce == expected_first {
            return true;
        }

        let Some(&first_nonce) = self.txs.keys().next() else {
            return false;
        };
        if first_nonce != expected_first {
            return false;
        }

        let mut expected = expected_first;
        for &nonce in self.txs.keys() {
            if nonce > target_nonce {
                break;
            }
            if nonce != expected {
                return false;
            }
            expected += 1;
        }

        expected > target_nonce
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
