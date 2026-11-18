use std::num::NonZero;

use rockbound::cache::delta_reader::DeltaReader;
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::{Runtime, Spec, StateCheckpoint, TxChangeSet, VisibleSlotNumber};
use sov_rollup_full_node_interface::StateUpdateInfo;
use sov_shutdown::PrimaryShutdownController;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};

use crate::common::AcceptedTx;
use crate::metrics::{
    track_in_progress_batch_size, MetricBatcher, PreferredSequencerExecutorEventSendingMetrics,
};
use crate::preferred::db::{BlobsCache, ReadBatch};
use crate::preferred::{
    exit_rollup, Confirmation, DbEvent, PreferredBatchToReplay, ReadBlob, RecoveryStrategy,
};
use crate::preferred::{PreferredBlobToReplay, PreferredProofToReplay};
use crate::{PreferredProofDataBytes, SlotNumber};

const MAX_EXECUTOR_EVENT_QUEUE_DEPTH: usize = 1000;

/// Sending metrics of accepted-tx events are submitted in batches of this many.
const METRICS_BATCH_SIZE: usize = 32;

/// The in-progress batch size gauge is reported at most once per this interval while txs
/// arrive, which keeps its submission off the per-tx path without hiding small batches.
const IN_PROGRESS_BATCH_SIZE_REPORT_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

pub(crate) struct ExecutorEventsSender<S: Spec, Rt: Runtime<S>> {
    events_sender: mpsc::Sender<ExecutorEvent<S, Rt>>,
    cache: BlobsCache,
    primary_shutdown: PrimaryShutdownController,
    /// Sending metrics of accepted-tx events, the only per-tx event; batched to keep submission
    /// off the per-tx path. Other events are rare and submit their metrics directly.
    accepted_tx_sending_metrics: MetricBatcher<PreferredSequencerExecutorEventSendingMetrics>,
    /// When the in-progress batch size gauge was last reported from `send_accept_tx`.
    last_batch_size_report: Option<std::time::Instant>,
}

impl<S: Spec, Rt: Runtime<S>> ExecutorEventsSender<S, Rt> {
    pub fn new(
        primary_shutdown: PrimaryShutdownController,
        cache: BlobsCache,
    ) -> (Self, mpsc::Receiver<ExecutorEvent<S, Rt>>) {
        let (sender, receiver) = mpsc::channel(MAX_EXECUTOR_EVENT_QUEUE_DEPTH);
        (
            Self {
                events_sender: sender,
                primary_shutdown,
                cache,
                accepted_tx_sending_metrics: MetricBatcher::new(METRICS_BATCH_SIZE),
                last_batch_size_report: None,
            },
            receiver,
        )
    }

    async fn shutdown_on_error(&self) {
        // The side effects task stops consuming events once it has triggered a shutdown after a
        // failure, so a closed channel is expected from then on. Exiting here would cut the
        // graceful shutdown short.
        if self.primary_shutdown.is_triggered() {
            tracing::debug!("Dropped an executor event because the rollup is shutting down");
            return;
        }
        tracing::error!("Failed to send executor event because the receiver was dropped. This indicates that the database is no longer available. Shutting down.");
        exit_rollup(&self.primary_shutdown).await;
    }

    /// [`Self::send_and_measure`], submitting the metrics right away.
    async fn send(&self, event: ExecutorEvent<S, Rt>) {
        let metrics = self.send_and_measure(event).await;
        sov_metrics::track_metrics(|t| {
            t.submit(metrics);
        });
    }

    /// Send an event and return metrics on the queue depth and blocking time, shutting down on
    /// error.
    async fn send_and_measure(
        &self,
        event: ExecutorEvent<S, Rt>,
    ) -> PreferredSequencerExecutorEventSendingMetrics {
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
        metrics
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
        // A gauge, so sampling it is enough. Closing the batch reports the reset to 0 directly.
        let now = std::time::Instant::now();
        if self
            .last_batch_size_report
            .is_none_or(|last| now.duration_since(last) >= IN_PROGRESS_BATCH_SIZE_REPORT_INTERVAL)
        {
            self.last_batch_size_report = Some(now);
            track_in_progress_batch_size(
                self.cache
                    .in_progress_batch_opt()
                    .map(|b| b.txs.len() as u64)
                    .unwrap_or(0),
            );
        }
        let metrics = self
            .send_and_measure(ExecutorEvent::AcceptedTx(AcceptedTxEventContents {
                accepted_tx,
                tx_changes,
                oneshot_sender: sender,
                sequence_number,
                tx_idx_within_batch,
            }))
            .await;
        self.accepted_tx_sending_metrics.push(metrics);
        receiver
    }

    /// Discards the soft confirmations served so far: the transaction cache is emptied and the
    /// API state falls back to the node's state in `info`, in one step, so that the events a
    /// client streams and the state snapshot it reads never disagree on the epoch.
    pub(crate) async fn flush_transactions_cache(&self, info: &StateUpdateInfo<S::Storage>) {
        // Note: we don't need to update the db cache here - this call is purely for the side effects task
        let (sender, receiver) = oneshot::channel();
        // Built ahead of the send, so that the kernel temporary does not live across the await.
        let checkpoint = StateCheckpoint::new(info.storage.clone(), &Rt::default().kernel());
        self.send(ExecutorEvent::FlushTransactionsCache {
            next_tx_number: info.next_tx_number,
            checkpoint,
            next_event_number: info.next_event_number,
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
        data: PreferredProofDataBytes,
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
        next_event_number: u64,
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
            next_event_number,
        })
        .await;
    }

    pub(crate) async fn close_batch(
        &mut self,
        checkpoint: StateCheckpoint<S>,
        forced_txs: Vec<AcceptedTx<Confirmation<S, Rt>>>,
        next_event_number: u64,
    ) {
        let batch = self.cache.terminate_batch().await;
        self.send(ExecutorEvent::CloseBatch {
            batch,
            checkpoint,
            forced_txs,
            next_event_number,
        })
        .await;
    }

    pub(crate) async fn prune(&mut self, prune_up_to_including: SequenceNumber) {
        self.cache.prune(prune_up_to_including).await;
        self.send(ExecutorEvent::PruneDb(prune_up_to_including))
            .await;
    }

    pub(crate) async fn force_update_api_state(
        &mut self,
        checkpoint: StateCheckpoint<S>,
        next_event_number: u64,
    ) {
        // No cache operation needed here - this is a side effect only.
        self.send(ExecutorEvent::ForceUpdateApiState(
            checkpoint,
            next_event_number,
        ))
        .await;
    }

    pub(crate) async fn update_api_ledger_from_info(&self, info: &StateUpdateInfo<S::Storage>) {
        self.send(ExecutorEvent::UpdateApiLedger {
            ledger_reader: info.ledger_reader.clone(),
            slot_number: info.slot_number,
            latest_finalized_slot_number: info.latest_finalized_slot_number,
            next_tx_number: info.next_tx_number,
        })
        .await;
    }

    /// Fetch the in-progress batch from the database.
    pub(crate) fn fetch_in_progress_batch(&self) -> Option<ReadBatch> {
        self.cache
            .in_progress_batch_opt()
            .cloned()
            .map(|b| b.into())
    }

    /// Fetch the in-progress batch from the database.
    pub(crate) fn fetch_proofs_for_replay(
        &self,
        sequence_number: SequenceNumber,
    ) -> Vec<PreferredProofToReplay> {
        self.cache.proofs_for_replay(sequence_number)
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

        // 2. Flush all completed batches and proofs to the BlobSender
        let blobs_to_flush = self
            .cache
            .all_proofs_and_completed_batches_greater_than_or_equal_to(
                next_sequence_number_according_to_node,
            );

        self.send(ExecutorEvent::TriggerRecovery {
            blobs_to_flush,
            recovery_strategy,
            batch_to_close,
        })
        .await;
    }

    pub(crate) async fn update_state_for_recovery(
        &mut self,
        checkpoint: StateCheckpoint<S>,
        next_event_number: u64,
    ) {
        // No cache operation needed here - this is a side effect only.
        self.send(ExecutorEvent::UpdateStateForRecovery(
            checkpoint,
            next_event_number,
        ))
        .await;
    }

    /// Fetches all proofs and any closed batches from the database that are greater than or equal to the given sequence number.
    /// Also includes the in-progress batch if `include_in_progress_batch` is true.
    /// Note that any proofs with sequence numbers greater than the in-progress batch will be included whether or not `include_in_progress_batch` is true.
    pub(crate) fn fetch_proofs_and_completed_batches_by_sequence(
        &self,
        after_and_including: SequenceNumber,
        include_in_progress_batch: bool,
    ) -> Vec<PreferredBlobToReplay> {
        let blobs_to_apply = self
            .cache
            .all_proofs_and_completed_batches_greater_than_or_equal_to(after_and_including);
        let first_sequence_number = blobs_to_apply.first().map(|b| b.sequence_number());

        tracing::trace!(
            blobs_count = blobs_to_apply.len(),
            first_sequence_number,
            last_sequence_number = blobs_to_apply.last().map(|b| b.sequence_number()),
            "Extracted blobs to apply from database"
        );

        let maybe_in_progress_batch = if include_in_progress_batch {
            self.cache.in_progress_batch_opt().cloned().map(|batch| {
                let batch: ReadBatch = batch.into();
                PreferredBatchToReplay {
                    is_in_progress: true,
                    visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
                    batch: batch.into_with_cached_tx_hashes(),
                }
            })
        } else {
            None
        };

        let mut blobs_to_replay = blobs_to_apply
            .into_iter()
            .map(|blob| match blob {
                ReadBlob::Batch(batch) => PreferredBlobToReplay::Batch(PreferredBatchToReplay {
                    is_in_progress: false,
                    visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
                    batch: batch.into_with_cached_tx_hashes(),
                }),
                ReadBlob::Proof {
                    sequence_number,
                    data,
                    ..
                } => {
                    tracing::trace!(sequence_number, "Processing proof blob");
                    PreferredBlobToReplay::Proof(PreferredProofToReplay {
                        sequence_number,
                        data,
                    })
                }
            })
            .chain(maybe_in_progress_batch.map(PreferredBlobToReplay::Batch))
            .collect::<Vec<_>>();

        blobs_to_replay.sort_by_key(|blob| blob.sequence_number());
        blobs_to_replay
    }

    pub fn clean_all_batches_from_cache(&mut self) {
        self.cache.clean_all_batches();
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
        /// The next event number reflected by `new_checkpoint`.
        next_event_number: u64,
    },
    /// Close the current batch.
    CloseBatch {
        batch: ReadBatch,
        checkpoint: StateCheckpoint<S>,
        forced_txs: Vec<AcceptedTx<Confirmation<S, Rt>>>,
        /// The next event number reflected by `checkpoint`.
        next_event_number: u64,
    },
    /// Publish a proof blob.
    PublishProofBlob(BlobInternalId, PreferredProofDataBytes, SequenceNumber),
    /// Insert an accepted transaction into the database and send out the confirmation
    AcceptedTx(AcceptedTxEventContents<S, Rt>),
    /// Update the API state to the given checkpoint without closing the current batch etc. Used during recovery.
    ///
    /// The `u64` is the next event number reflected by the checkpoint. It can move *backwards*
    /// relative to the previously published checkpoint when the sequencer discards its
    /// soft confirmations and adopts the node's state.
    ForceUpdateApiState(StateCheckpoint<S>, u64),
    /// Update the ledger reader and send slot notifications for API/WebSocket consistency.
    UpdateApiLedger {
        ledger_reader: DeltaReader,
        slot_number: SlotNumber,
        latest_finalized_slot_number: SlotNumber,
        next_tx_number: u64,
    },
    /// Prune the database up to the given sequence number.
    PruneDb(SequenceNumber),
    /// Enter recovery mode.
    TriggerRecovery {
        #[allow(missing_docs)]
        blobs_to_flush: Vec<ReadBlob>,
        #[allow(missing_docs)]
        recovery_strategy: RecoveryStrategy,
        #[allow(missing_docs)]
        batch_to_close: Option<ReadBatch>,
    },
    /// During recovery mode, we periodically update the state to the node's state.
    ///
    /// The `u64` is the next event number reflected by the checkpoint; like
    /// [`Self::ForceUpdateApiState`] it can move backwards.
    UpdateStateForRecovery(StateCheckpoint<S>, u64),
    /// Flush the transactions cache and publish the node's state it is being rebuilt from.
    ///
    /// The `u64` is the next event number reflected by the checkpoint; like
    /// [`Self::ForceUpdateApiState`] it can move backwards.
    FlushTransactionsCache {
        next_tx_number: u64,
        checkpoint: StateCheckpoint<S>,
        next_event_number: u64,
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

#[cfg(test)]
mod tests {
    use sov_test_utils::{generate_optimistic_runtime, TestSpec as S};

    use super::*;

    generate_optimistic_runtime!(TestRuntime <= );

    /// The side effects task closes the channel once it has triggered a shutdown after a
    /// failure. Senders must put up with that instead of forcing the process to exit.
    #[tokio::test]
    async fn test_sending_to_a_closed_channel_is_tolerated_while_shutting_down() {
        let primary_shutdown = PrimaryShutdownController::new();
        let (mut sender, receiver) = ExecutorEventsSender::<S, TestRuntime<S>>::new(
            primary_shutdown.clone(),
            BlobsCache::new(Default::default(), None, primary_shutdown.clone()),
        );
        primary_shutdown.shutdown();
        drop(receiver);

        // A sender that does not tolerate the closed channel never returns: it waits a few
        // seconds for logs to be flushed and then exits the process.
        let send_result =
            tokio::time::timeout(std::time::Duration::from_secs(1), sender.prune(0x1234)).await;

        assert!(
            send_result.is_ok(),
            "Sending to a closed channel should return straight away while shutting down"
        );
    }
}
