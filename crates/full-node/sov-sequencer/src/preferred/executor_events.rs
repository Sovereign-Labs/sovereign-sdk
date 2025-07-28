use std::num::NonZero;
use std::sync::Arc;

use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::{
    FullyBakedTx, Runtime, Spec, StateCheckpoint, TxChangeSet, TxHash, VisibleSlotNumber,
};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot, watch};

use crate::common::AcceptedTx;
use crate::metrics::{track_in_progress_batch_size, PreferredSequencerExecutorEventSendingMetrics};
use crate::preferred::db::{PreferredSequencerCache, PreferredSequencerReadBatch};
use crate::preferred::{
    exit_rollup, Confirmation, DbEvent, PreferredBatchToReplay, PreferredSequencerReadBlob,
    RecoveryStrategy,
};

const MAX_EXECUTOR_EVENT_QUEUE_DEPTH: usize = 1000;

pub(crate) struct ExecutorEventsSender<S: Spec, Rt: Runtime<S>> {
    events_sender: mpsc::Sender<ExecutorEvent<S, Rt>>,
    cache: PreferredSequencerCache,
    shutdown_sender: watch::Sender<()>,
}

impl<S: Spec, Rt: Runtime<S>> ExecutorEventsSender<S, Rt> {
    pub fn new(
        shutdown_sender: watch::Sender<()>,
        cache: PreferredSequencerCache,
    ) -> (Self, mpsc::Receiver<ExecutorEvent<S, Rt>>) {
        let (sender, receiver) = mpsc::channel(MAX_EXECUTOR_EVENT_QUEUE_DEPTH);
        (
            Self {
                events_sender: sender,
                shutdown_sender,
                cache,
            },
            receiver,
        )
    }

    async fn shutdown_on_error(&self) {
        tracing::error!("Failed to send executor event because the receiver was dropped. This indicates that the database is no longer available. Shutting down.");
        exit_rollup(&self.shutdown_sender).await;
    }

    /// Send an event tracking metrics on the queue depth and blocking time and shutting down on error.
    async fn send(&self, event: ExecutorEvent<S, Rt>) {
        let mut metrics = PreferredSequencerExecutorEventSendingMetrics::default();
        match self.events_sender.try_send(event) {
            Ok(()) => (),
            Err(TrySendError::Full(event)) => {
                tracing::trace!(
                    "Executor event channel is full. Blocking until it becomes available again."
                );
                let started_blocking = std::time::Instant::now();
                if self.events_sender.send(event).await.is_err() {
                    self.shutdown_on_error().await;
                };
                metrics.blocked_for_us = started_blocking.elapsed().as_micros() as u64;
            }
            Err(TrySendError::Closed(_)) => self.shutdown_on_error().await,
        }

        let queue_depth = self.events_sender.max_capacity() - self.events_sender.capacity();
        metrics.queue_depth = queue_depth;
        sov_metrics::track_metrics(|t| {
            t.submit(metrics);
        });
    }

    /// Send a notification of an accepted tx. Return a receiver that will receive the confirmation.
    pub(crate) async fn send_accept_tx(
        &mut self,
        accepted_tx: AcceptedTx<Confirmation<S, Rt>>,
        tx_changes: TxChangeSet,
        sequence_number: SequenceNumber,
    ) -> oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>> {
        let tx_idx_within_batch = self
            .cache
            .in_progress_batch_opt()
            .map(|b| b.txs.len())
            .unwrap_or(0) as u64;

        self.cache
            .insert_tx(accepted_tx.tx.clone(), accepted_tx.tx_hash)
            .await;

        let (sender, receiver) = oneshot::channel();
        track_in_progress_batch_size(
            self.cache
                .in_progress_batch_opt()
                .map(|b| b.txs.len() as u64)
                .unwrap_or(0),
        );
        self.send(ExecutorEvent::AcceptedTx(AcceptedTxEventContents {
            accepted_tx,
            tx_changes,
            oneshot_sender: sender,
            sequence_number,
            tx_idx_within_batch,
        }))
        .await;
        receiver
    }

    pub(crate) async fn flush_transactions_cache(&self, next_tx_number: u64) {
        // Note: we don't need to update the db cache here - this call is purely for the side effects task
        let (sender, receiver) = oneshot::channel();
        self.send(ExecutorEvent::FlushTransactionsCache {
            next_tx_number,
            oneshot_sender: sender,
        })
        .await;
        if receiver.await.is_err() {
            tracing::error!(
                "Failed to flush transactions cache because the side effects task is no longer available."
            );
            self.shutdown_on_error().await;
        };
    }

    pub(crate) async fn publish_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        sequence_number: SequenceNumber,
    ) {
        self.cache
            .insert_proof_blob(blob_id, data.clone(), sequence_number)
            .await;
        self.send(ExecutorEvent::PublishProofBlob(
            blob_id,
            data,
            sequence_number,
        ))
        .await;
    }

    pub(crate) async fn start_batch(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
        sequence_number: SequenceNumber,
        new_checkpoint: StateCheckpoint<S>,
    ) {
        let blob_id = self
            .cache
            .start_batch(
                visible_slot_number_after_increase,
                visible_slots_to_advance,
                sequence_number,
            )
            .await;
        self.send(ExecutorEvent::StartBatch {
            visible_slot_number_after_increase,
            visible_slots_to_advance,
            sequence_number,
            new_checkpoint,
            blob_id,
        })
        .await;
    }

    pub(crate) async fn close_batch(&mut self, checkpoint: StateCheckpoint<S>) {
        let batch = self.cache.terminate_batch().await;
        self.send(ExecutorEvent::CloseBatch(batch, checkpoint))
            .await;
    }

    pub(crate) async fn prune(&mut self, prune_up_to_including: SequenceNumber) {
        self.cache.prune(prune_up_to_including).await;
        self.send(ExecutorEvent::PruneDb(prune_up_to_including))
            .await;
    }

    pub(crate) async fn force_update_api_state(&mut self, checkpoint: StateCheckpoint<S>) {
        // No cache operation needed here - this is a side effect only.
        self.send(ExecutorEvent::ForceUpdateApiState(checkpoint))
            .await;
    }

    /// Fetch the in-progress batch from the database.
    pub(crate) fn fetch_in_progress_batch(&self) -> Option<PreferredSequencerReadBatch> {
        self.cache
            .in_progress_batch_opt()
            .cloned()
            .map(|b| b.into())
    }

    pub(crate) fn subscribe_to_events(&mut self, sender: mpsc::Sender<DbEvent>) {
        self.cache.subscribe_to_events(sender);
    }

    pub(crate) async fn trigger_recovery(
        &mut self,
        next_sequence_number_according_to_node: SequenceNumber,
        recovery_strategy: RecoveryStrategy,
    ) {
        tracing::trace!("Recovery: flushing all preferred sequencer batches");
        // 1. close the in-progress batch, if any
        let batch_to_close = if self.cache.in_progress_batch_opt().is_some() {
            tracing::debug!("Recovery: In-progress batch found, terminating it.");
            Some(self.cache.terminate_batch().await)
            // No need to update API state, we're going to overwrite it with the node's state soon
        } else {
            tracing::debug!("Recovery: No in-progress batch to terminate.");
            None
        };

        // 2. Flush all batches to the BlobSender
        let blobs_to_flush = self
            .cache
            .all_completed_blobs_greater_than_or_equal_to(next_sequence_number_according_to_node);

        self.send(ExecutorEvent::TriggerRecovery {
            blobs_to_flush,
            recovery_strategy,
            batch_to_close,
        })
        .await;
    }

    pub(crate) async fn insert_tx_without_confirmation(
        &mut self,
        tx: FullyBakedTx,
        tx_hash: TxHash,
    ) {
        self.cache.insert_tx(tx, tx_hash).await;
    }

    pub(crate) async fn update_state_for_recovery(&mut self, checkpoint: StateCheckpoint<S>) {
        // No cache operation needed here - this is a side effect only.
        self.send(ExecutorEvent::UpdateStateForRecovery(checkpoint))
            .await;
    }

    pub(crate) fn fetch_completed_blobs_by_sequence(
        &self,
        after_and_including: SequenceNumber,
        include_in_progress_batch: bool,
    ) -> Vec<PreferredBatchToReplay> {
        let blobs_to_apply = self
            .cache
            .all_completed_blobs_greater_than_or_equal_to(after_and_including);
        let first_sequence_number = blobs_to_apply.first().map(|b| b.sequence_number());

        tracing::trace!(
            blobs_count = blobs_to_apply.len(),
            first_sequence_number,
            last_sequence_number = blobs_to_apply.last().map(|b| b.sequence_number()),
            "Extracted blobs to apply from database"
        );

        let maybe_in_progress_batch = if include_in_progress_batch {
            self.cache.in_progress_batch_opt().cloned().map(|batch| {
                let batch: PreferredSequencerReadBatch = batch.into();
                PreferredBatchToReplay {
                    is_in_progress: true,
                    visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
                    batch: batch.into_with_cached_tx_hashes(),
                }
            })
        } else {
            None
        };

        blobs_to_apply
            .into_iter()
            .filter_map(|blob| match blob {
                PreferredSequencerReadBlob::Batch(batch) => Some(PreferredBatchToReplay {
                    is_in_progress: false,
                    visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
                    batch: batch.into_with_cached_tx_hashes(),
                }),
                // TODO(https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2063): Process proofs.
                // Note: once we start processing proofs in addition to batches,
                // we gotta make sure to order everything by sequence number as
                // proofs can have a sequence number that's greater than the
                // in-progress batch.
                _ => {
                    tracing::trace!(
                        sequence_number = %blob.sequence_number(),
                        "Ignoring proof blob"
                    );
                    None
                }
            })
            .chain(maybe_in_progress_batch)
            .collect::<Vec<_>>()
    }
}

#[derive(strum::IntoStaticStr)]
pub(crate) enum ExecutorEvent<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    /// Start a new batch.
    StartBatch {
        #[allow(missing_docs)]
        visible_slot_number_after_increase: VisibleSlotNumber,
        #[allow(missing_docs)]
        visible_slots_to_advance: NonZero<u8>,
        #[allow(missing_docs)]
        sequence_number: SequenceNumber,
        #[allow(missing_docs)]
        new_checkpoint: StateCheckpoint<S>,
        #[allow(missing_docs)]
        blob_id: BlobInternalId,
    },
    /// Close the current batch.
    CloseBatch(PreferredSequencerReadBatch, StateCheckpoint<S>),
    /// Publish a proof blob.
    PublishProofBlob(BlobInternalId, Arc<[u8]>, SequenceNumber),
    /// Insert an accepted transaction into the database and send out the confirmation
    AcceptedTx(AcceptedTxEventContents<S, Rt>),
    /// Update the API state to the given checkpoint without closing the current batch etc. Used during recovery
    ForceUpdateApiState(StateCheckpoint<S>),
    /// Prune the database up to the given sequence number.
    PruneDb(SequenceNumber),
    /// Enter recovery mode.
    TriggerRecovery {
        #[allow(missing_docs)]
        blobs_to_flush: Vec<PreferredSequencerReadBlob>,
        #[allow(missing_docs)]
        recovery_strategy: RecoveryStrategy,
        #[allow(missing_docs)]
        batch_to_close: Option<PreferredSequencerReadBatch>,
    },
    /// During recovery mode, we periodically update the state to the node's state.
    UpdateStateForRecovery(StateCheckpoint<S>),
    /// Flush transactions cache
    FlushTransactionsCache {
        next_tx_number: u64,
        oneshot_sender: oneshot::Sender<()>,
    },
}

pub(crate) struct AcceptedTxEventContents<S: Spec, Rt: Runtime<S>> {
    pub accepted_tx: AcceptedTx<Confirmation<S, Rt>>,
    pub tx_changes: TxChangeSet,
    pub oneshot_sender: oneshot::Sender<AcceptedTx<Confirmation<S, Rt>>>,
    pub sequence_number: SequenceNumber,
    pub tx_idx_within_batch: u64,
}
