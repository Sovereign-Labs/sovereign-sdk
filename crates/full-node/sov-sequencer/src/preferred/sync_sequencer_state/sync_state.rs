use crate::metrics::{PreferredSequencerPruneMetrics, PreferredSequencerSlotNumberMetrics};
use crate::preferred::block_executor::{RollupBlockExecutor, RollupBlockExecutorError};
use crate::preferred::db::BatchToStore;
use crate::preferred::preferred_blob_sender::proof_bytes;
use crate::preferred::rate_limiter::IpAndCredentialId;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::event_handler::ReplicaError;
use crate::preferred::replica::replica_sync_task::DBDataRejected;
use crate::preferred::sync_sequencer_state::conditions_table::{
    operation_for_master, operation_for_replica,
};
use crate::preferred::sync_sequencer_state::ConditionsTable;
use crate::preferred::sync_sequencer_state::{InitialStatus, Message};
use crate::preferred::update_state::do_next_event;
use crate::preferred::update_state::SequenceNumberMismatchError;
use crate::preferred::DoNewTxError;
use crate::preferred::Inner;
use crate::preferred::InnerGuard;
use crate::preferred::PreferredProofToReplay;
use crate::preferred::ProcessFinalCatchupData;
use crate::preferred::SequencerStateUpdatorError;
use crate::preferred::StateUpdateNotification;
use crate::preferred::{
    current_visible_slot_number_according_to_node, get_next_sequence_number_according_to_node,
    slot_count_delta_acceptable_lower_bound, AcceptedTx, Confirmation, DbEvent,
    PreferredSeqOperation, PreferredSequencerFetchBatchesToReplayMetrics, ReadBatch,
};
use crate::preferred::{AcceptTxError, PreferredBlobToReplay};
use crate::{
    PreferredProofDataBytes, SequencerNotReadyDetails, SerializedProofWithDetailsBytes, TxHash,
};
use sov_blob_sender::{new_blob_id, BlobInternalId};
use sov_blob_storage::SequenceNumber;
use sov_modules_api::capabilities::{RollupHeight, SequencingDataHandler};
use sov_modules_api::state::{ApiStateAccessor, ConcurrentStateCheckpoint};
use sov_modules_api::{
    FullyBakedTx, Runtime, Spec, StateCheckpoint, StateUpdateInfo, VersionReader,
};
use sov_state::Storage;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::debug;

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Priority {
    priority: u64,
    index: u64,
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let priority_cmp = self.priority.cmp(&other.priority);
        if priority_cmp == std::cmp::Ordering::Equal {
            // If priority is equal, give higher priority to the message that is older
            self.index.cmp(&other.index).reverse()
        } else {
            priority_cmp
        }
    }
}

pub(crate) struct SynchronizedSequencerState<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub(super) inner: Inner<S, Rt>,
    pub(super) channel_size: Arc<AtomicU32>,
    pub(super) message_receiver: mpsc::Receiver<Message<S, Rt>>,
    // A heap of message, ordered from low to high priority.
    pub(super) heap: BTreeMap<Priority, Message<S, Rt>>,
    pub(super) runtime: Rt,
    pub(crate) test_only_state_update_notification_sender:
        broadcast::Sender<StateUpdateNotification>,
}

impl<S, Rt> SynchronizedSequencerState<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    const MAX_HEAP_SIZE: usize = 10_000;

    async fn send_response<T>(&mut self, resp: oneshot::Sender<T>, v: T, name: &'static str) {
        if resp.send(v).is_err() {
            tracing::debug!("SynchronizedSequencerState: Response channel closed - unable to send response to {}", name);
        }
    }

    fn heap_is_full(&self) -> bool {
        self.heap.len() >= Self::MAX_HEAP_SIZE
    }

    pub(crate) async fn start(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut index = 0;
            loop {
                // Start by trying to drain the channel of inbound messages.
                while let Ok(msg) = self.message_receiver.try_recv() {
                    self.heap.insert(
                        Priority {
                            priority: msg.priority(&self.runtime),
                            index,
                        },
                        msg,
                    );
                    index += 1;

                    // If the heap is full after adding the new message, try to clear some space by dropping old messages
                    if self.heap_is_full() {
                        tracing::warn!("SynchronizedSequencerState: The message heap is full, dropping the lowest priority message if possible.");
                        let Some((priority, lowest_priority_msg)) = self.heap.pop_first() else {
                            // This should be unreachable - we just checked that the heap was full
                            unreachable!("A heap with len >= 10_000 return None for pop_first");
                        };

                        // If the lowest priority message is an accept_tx message, send a 503 and drop it. Now we can continue retrieving messages from the channel.
                        // Here we rely on the guarantee that accept_tx messages always have lower priority than other message types. If this guarantee is removed later,
                        // we'll need to update this logic.
                        if let Message::AcceptTx { resp, .. } = lowest_priority_msg {
                            self.send_response(
                                resp,
                                Err(AcceptTxError::SequencerOverloaded503),
                                "accept_tx",
                            )
                            .await;
                        } else {
                            // Otherwise, the heap is completely full of undroppable messages. Stop reading from the channel and process one.
                            self.heap.insert(priority, lowest_priority_msg);
                            break;
                        }
                    }
                }

                // Process the highest priority message in the heap. This ensures that the heap will not be full at the start of the next iteration
                if let Some((_priority, msg)) = self.heap.pop_last() {
                    if let Err(e) = self.handle_next_message(msg).await {
                        match e {
                            SequencerStateUpdatorError::Shutdown => {
                                return;
                            }
                            SequencerStateUpdatorError::Unexpected => {
                                self.inner.shutdown_sender.send(()).unwrap();
                                panic!("The sequencer experienced an unexpected error and cannot accept transactions! See logs for more details.");
                            }
                        }
                    }
                }

                // If we don't have any more messages to process, yield until a message becomes available
                if self.heap.is_empty() {
                    let Some(msg) = self.message_receiver.recv().await else {
                        break;
                    };
                    assert!(
                        self.heap
                            .insert(
                                Priority {
                                    priority: msg.priority(&self.runtime),
                                    index
                                },
                                msg
                            )
                            .is_none(),
                        "Duplicate priority for message. This is a bug, please report it"
                    );
                    index += 1;
                }
            }
        })
    }

    async fn handle_next_message(
        &mut self,
        msg: Message<S, Rt>,
    ) -> Result<(), SequencerStateUpdatorError> {
        // We intentionally don't check for shutdown at the beginning of the loop.
        // Each `process_xx` method handles shutdown internally.
        match msg {
            Message::NextSequenceNumber { resp, reason } => {
                let ret = self.process_next_sequence_number(reason).await;
                self.send_response(resp, ret, "next_sequence_number").await;
            }

            Message::FetchProofsAndCompletedBatches {
                resp,
                next_sequence_number,
                reason,
            } => {
                let ret = self
                    .process_fetch_proofs_and_completed_batches(next_sequence_number, reason)
                    .await;

                self.send_response(resp, ret, "fetch_completed_batches")
                    .await;
            }
            Message::SequencerConditions {
                resp,
                info,
                next_sequence_number_according_to_node,
                reason,
            } => {
                let ret = self
                    .process_sequencer_conditions(
                        &info,
                        next_sequence_number_according_to_node,
                        reason,
                    )
                    .await;

                self.send_response(resp, ret, "sequencer_conditions").await;
            }
            Message::CheckReadiness {
                resp,
                max_concurrent_blobs,
                height_to_stop_at,
                reason,
            } => {
                let ret = self
                    .process_check_readiness(max_concurrent_blobs, height_to_stop_at, reason)
                    .await;

                self.send_response(resp, ret, "check_readiness").await;
            }
            Message::AcceptTx {
                resp,
                baked_tx,
                tx_hash,
                original_tx_queue_id,
                ip_and_credential,
                reason,
            } => {
                let ret = self
                    .process_accept_tx(
                        baked_tx,
                        tx_hash,
                        original_tx_queue_id,
                        ip_and_credential,
                        reason,
                    )
                    .await;
                if let Err(AcceptTxError::NewTxError(DoNewTxError::ExecutorError(
                    RollupBlockExecutorError::UnexpectedFailure,
                ))) = ret
                {
                    // Propagate the error to the spawner of this task.
                    panic!("Unexpected in the rollup block executor. The sequencer can no longer accept transactions!");
                }

                self.send_response(resp, ret, "accept_tx").await;
            }

            Message::FinalCatchup {
                resp,
                info,
                db_event_subscription,
                executor,
                node_state_root,
                data,
                reason,
            } => {
                let slot_number = info.slot_number;
                let finalized_slot_number = info.latest_finalized_slot_number;
                let ret = self
                    .process_final_catchup(
                        info,
                        db_event_subscription,
                        executor,
                        node_state_root,
                        data,
                        reason,
                    )
                    .await;

                self.send_response(resp, ret, "final_catchup").await;
                // Send a state update notification (for testing. Note that we've already released the lock at this point, so there should be no performance impact)
                // but updates are not strictly guaranteed to be delivered in order. We discard errors because we don't care if there are no subscribers.
                let _ =
                    self.test_only_state_update_notification_sender
                        .send(StateUpdateNotification {
                            slot_number,
                            finalized_slot_number,
                            #[cfg(feature = "test-utils")]
                            update_skipped_due_to_pause: false,
                            #[cfg(feature = "test-utils")]
                            triggered_recovery: false,
                        });
            }
            Message::PruneSequencerDb { reason } => {
                self.process_prune_sequencer_db(reason).await;
            }
            Message::ForceOverwriteStateForRecovery { info, reason } => {
                self.process_force_overwrite_state_for_recovery(info, reason)
                    .await;
            }
            Message::WaitNodeResync {
                info,
                distance,
                reason,
            } => {
                self.process_wait_for_node_resync(info, distance, reason)
                    .await;
            }
            #[cfg(feature = "test-utils")]
            Message::ForceCloseCurrentBatch {
                reason: _reason,
                result_sender,
            } => {
                self.process_force_close_current_batch(_reason, result_sender)
                    .await;
            }
            Message::ProofBlob {
                blob_id,
                data,
                reason,
            } => self.process_proof_blob(blob_id, data, reason).await,
            Message::TriggerBatchProduction { reason } => {
                self.process_trigger_batch_production(reason).await;
            }
            Message::SimpleStateUpdate { info } => {
                let slot_number = info.slot_number;
                let finalized_slot_number = info.latest_finalized_slot_number;

                self.process_new_storage(info).await;
                // Send a state update notification (for testing. Note that we've already released the lock at this point, so there should be no performance impact)
                // but updates are not strictly guaranteed to be delivered in order. We discard errors because we don't care if there are no subscribers.
                let _ =
                    self.test_only_state_update_notification_sender
                        .send(StateUpdateNotification {
                            slot_number,
                            finalized_slot_number,
                            #[cfg(feature = "test-utils")]
                            update_skipped_due_to_pause: false,
                            #[cfg(feature = "test-utils")]
                            triggered_recovery: false,
                        });
            }
            Message::ReplicaBatchStartMsg {
                resp,
                batch_from_master,
                reason,
            } => {
                let ret = self
                    .process_do_batch_start_replica(batch_from_master, reason)
                    .await;

                self.send_response(resp, ret, "process_do_batch_start_replica")
                    .await;
            }
            Message::ReplicaNewTx {
                resp,
                seq_nr_from_master,
                tx_hash,
                baked_tx,
                reason,
            } => {
                let ret = self
                    .process_do_new_tx_replica(seq_nr_from_master, baked_tx, tx_hash, reason)
                    .await;

                self.send_response(resp, ret, "process_do_new_tx_replica")
                    .await;
            }
            Message::ReplicaCloseCurrentBatch {
                resp,
                batch_from_master,
                reason,
            } => {
                let ret = self
                    .process_close_current_batch_replica(batch_from_master, reason)
                    .await;
                self.send_response(resp, ret, "process_do_batch_start_replica")
                    .await;
            }
            Message::ReplicaNewProof {
                resp,
                sequence_number,
                proof_bytes,
                reason,
            } => {
                let ret = self
                    .process_new_proof_replica(sequence_number, proof_bytes, reason)
                    .await;
                self.send_response(resp, ret, "process_new_proof_replica")
                    .await;
            }
            Message::GetSequencerRole { resp, reason } => {
                let inner = self.get_inner_with_timing(reason).await;
                let role = inner.seq_role;
                drop(inner);
                self.send_response(resp, role, "get_sequencer_role").await;
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "debug")]
    async fn get_inner_with_timing(&mut self, reason: &'static str) -> InnerGuard<'_, S, Rt> {
        let channel_size = self.channel_size.fetch_sub(1, Ordering::Relaxed);
        InnerGuard::new(&mut self.inner, reason, channel_size)
    }

    async fn process_next_sequence_number(&mut self, reason: &'static str) -> SequenceNumber {
        let inner = self.get_inner_with_timing(reason).await;
        inner.next_unassigned_sequence_number
    }

    async fn process_fetch_proofs_and_completed_batches(
        &mut self,
        next_sequence_number: u64,
        reason: &'static str,
    ) -> FetchProofsAndCompletedBatches {
        let mut inner = self.get_inner_with_timing(reason).await;

        let (completed_blobs, metrics) =
            inner.proofs_and_completed_batches_for_replay(next_sequence_number, false);
        let has_completed_batch = completed_blobs_contain_batch(&completed_blobs);

        // Once we've caught up to the in-progress batch, we're done.
        let (db_events_sender, subscription) =
            mpsc::channel(inner.seq_config.sequencer_kind_config.db_event_channel_size);
        if !has_completed_batch {
            inner
                .executor_events_sender
                .subscribe_to_events(db_events_sender);

            let fetch_in_progress_batch_time_start = std::time::Instant::now();
            let in_progress_batch = inner.executor_events_sender.fetch_in_progress_batch();
            let fetch_in_progress_batch_time = fetch_in_progress_batch_time_start.elapsed();
            let pending_completed_proofs = completed_blobs
                .into_iter()
                .filter_map(|blob| match blob {
                    PreferredBlobToReplay::Batch(_) => None,
                    PreferredBlobToReplay::Proof(proof) => Some(proof),
                })
                .collect::<Vec<_>>();

            if !pending_completed_proofs.is_empty() {
                debug!(
                    pending_completed_proofs = pending_completed_proofs.len(),
                    next_sequence_number,
                    "Completed proofs found without a completed batch; carrying them into final catchup",
                );
            }

            drop(inner);
            return FetchProofsAndCompletedBatches {
                metrics,
                flow: Flow::Break {
                    pending_completed_proofs,
                    in_progress_batch,
                    subscription,
                    fetch_in_progress_batch_time,
                },
            };
        }

        drop(inner);
        FetchProofsAndCompletedBatches {
            metrics,
            flow: Flow::Continue { completed_blobs },
        }
    }

    async fn process_sequencer_conditions(
        &mut self,
        info: &StateUpdateInfo<S::Storage>,
        next_sequence_number_according_to_node: u64,
        reason: &'static str,
    ) -> PreferredSeqOperation<S, Rt> {
        let sync_status = &info.sync_status;
        let true_slot_number = info.slot_number.get();
        let latest_finalized_slot_number = info.latest_finalized_slot_number.get();
        let node_visible_slot_number =
            current_visible_slot_number_according_to_node::<S, Rt>(info).get();

        debug!(?info, "Processing state update info from update_state");
        let mut inner = self.get_inner_with_timing(reason).await;
        let next_sequence_number = inner.next_unassigned_sequence_number;
        let ((blobs_to_replay, fetch_batches_to_replay_metrics), is_startup) = {
            (
                inner.proofs_and_completed_batches_for_replay(
                    next_sequence_number_according_to_node,
                    true,
                ),
                !inner.has_finished_startup,
            )
        };

        let seq_visible_slot_number = match blobs_to_replay
            .iter()
            .filter_map(|b| match b {
                PreferredBlobToReplay::Batch(b) => Some(b),
                PreferredBlobToReplay::Proof(_) => None,
            })
            .next_back()
        {
            None => node_visible_slot_number,
            Some(b) => {
                let _visible_slots_advance = b.batch.inner.visible_slots_to_advance.get();
                b.visible_slot_number_after_increase.get()
            }
        };

        let is_resync = matches!(
            inner.is_ready,
            Err(SequencerNotReadyDetails::Syncing { .. })
        );

        let is_recover = matches!(
            inner.is_ready,
            Err(SequencerNotReadyDetails::PreferredSequencerRecovering)
        );

        let time_spent_fetching_batches = fetch_batches_to_replay_metrics.duration;
        sov_metrics::track_metrics(|t| {
            t.submit(fetch_batches_to_replay_metrics);
            t.submit(PreferredSequencerSlotNumberMetrics {
                true_slot_number,
                latest_finalized_slot_number,
                node_visible_slot_number,
                seq_visible_slot_number,
            });
        });

        let distance = sync_status.distance();

        let nodes_sequence_number_is_fresher =
            next_sequence_number_according_to_node > next_sequence_number;

        // There's an edge case on restart where the node hasn't synced to the chain tip yet but doesn't know it. We can check for it by seeing
        // if the first blob to replay has a sequencer number that's more than 1 greater than the node's sequence number (because sequence numbers are contiguous, and
        // we only prune a blob from the sequencer DB after it's been finalized on DA - so that blob must already exist on DA and the node just doesn't know that it isn't synced).
        let oldest_unfinalized_sequence_number = blobs_to_replay.first().map(|b| match b {
            PreferredBlobToReplay::Batch(b) => b.batch.inner.sequence_number,
            PreferredBlobToReplay::Proof(p) => p.sequence_number,
        });
        let node_is_unsynced_and_doesnt_know_it = (next_sequence_number_according_to_node
            .saturating_add(1))
            < oldest_unfinalized_sequence_number.unwrap_or(0);

        // Once we're this close to `deferred_slots_count`, we risk crossing the
        // `deferred_slots_count` threshold before the next call to
        // `update_state`. That's no good.
        let current_visible_slot_number =
            current_visible_slot_number_according_to_node::<S, Rt>(info);
        let too_close_to_deferred_slots_count_for_comfort =
            info.slot_number.delta(current_visible_slot_number)
                > slot_count_delta_acceptable_lower_bound(
                    inner.seq_config.max_allowed_node_distance_behind,
                );

        // Resuming operations while the node is
        // lagging can cause issues e.g. during failover or after sequencer DB
        // deletion due to in-flight blobs that are not yet processed.
        let node_is_lagging = distance > inner.seq_config.max_allowed_node_distance_behind;

        // Are there ANY soft confirmations to replay at all?
        // Note that we're holding a lock on the sequencer, so this is guaranteed to be up to date.
        let are_there_batches_to_replay = completed_blobs_contain_batch(&blobs_to_replay);

        let table = ConditionsTable {
            nodes_sequence_number_is_fresher,
            too_close_to_deferred_slots_count_for_comfort,
            node_is_lagging,
            are_there_batches_to_replay,
            node_is_unsynced_and_doesnt_know_it,
        };

        let initial_status = InitialStatus {
            is_startup,
            is_resync,
            is_recover,
        };

        if inner.is_replica_role() {
            operation_for_replica(
                table,
                info,
                &mut inner,
                initial_status,
                time_spent_fetching_batches,
                current_visible_slot_number,
            )
            .await
        } else {
            operation_for_master(
                table,
                info,
                &mut inner,
                initial_status,
                time_spent_fetching_batches,
                current_visible_slot_number,
            )
            .await
        }
    }

    async fn process_check_readiness(
        &mut self,
        max_concurrent_blobs: usize,
        height_to_stop_at: Option<RollupHeight>,
        reason: &'static str,
    ) -> Result<(), SequencerNotReadyDetails> {
        let inner = self.get_inner_with_timing(reason).await;
        inner
            .check_readiness(max_concurrent_blobs, height_to_stop_at)
            .await
    }

    async fn process_new_storage(&mut self, info: StateUpdateInfo<S::Storage>) {
        let mut inner = self.get_inner_with_timing("update_state::fast_path").await;
        // Atomically swap in the new storage and prune the old one.
        // Note that we use `StateCheckpoint::new(info.storage.clone(), ...)` *without* passing any intermediate state. This
        // is because we want to see what the height of the checkpoint we just received is, not the height of the sequencer's intermediate state.
        let mut rt = Rt::default();
        let new_rollup_height = StateCheckpoint::new(info.storage.clone(), &rt.kernel(), None)
            .rollup_height_to_access();

        inner
            .executor
            .uncommitted_changes
            .prune_changes_through(new_rollup_height.get());
        let uncommitted_changes = inner.executor.uncommitted_changes.clone();
        inner
            .executor
            .checkpoint
            .replace_storage(info.storage.clone(), Box::new(uncommitted_changes));
        tracing::debug!(%new_rollup_height, "Storage has been replaced");

        Self::common_for_final_catchup_and_new_storage(&mut inner, info.clone()).await;

        // Compute finalized_rollup_height from the finalized slot to avoid over-pruning during reorgs.
        // Only prune state roots for heights that are finalized on the DA layer.
        let finalized_rollup_height = {
            let concurrent_checkpoint = Arc::new(ConcurrentStateCheckpoint::from_state_checkpoint(
                StateCheckpoint::new(info.storage.clone(), &rt.kernel(), None),
            ));
            let kernel_with_slot_mapping = rt.kernel_with_slot_mapping();

            match ApiStateAccessor::new_archival_with_true_slot_number(
                concurrent_checkpoint,
                kernel_with_slot_mapping.clone(),
                info.latest_finalized_slot_number,
            ) {
                Ok(mut api_state) => kernel_with_slot_mapping.current_rollup_height(&mut api_state),
                Err(e) => {
                    // Fallback: if archival access fails, don't prune to avoid over-pruning
                    tracing::warn!(
                        ?e,
                        "Failed to get finalized rollup height, skipping state_roots pruning"
                    );
                    return;
                }
            }
        };

        inner
            .executor
            .state_roots
            .retain(|height, _| *height > finalized_rollup_height);
    }

    async fn process_final_catchup(
        &mut self,
        info: StateUpdateInfo<S::Storage>,
        mut db_event_subscription: mpsc::Receiver<DbEvent>,
        mut executor: Box<RollupBlockExecutor<S, Rt>>,
        node_state_root: <S::Storage as Storage>::Root,
        mut data: ProcessFinalCatchupData,
        reason: &'static str,
    ) -> Result<ProcessFinalCatchupData, SequenceNumberMismatchError> {
        let mut inner = self.get_inner_with_timing(reason).await;
        let tx_cache_writer = inner.tx_cache_writer.clone();

        let mut rt = Rt::default();
        let next_sequence_number_according_to_node =
            get_next_sequence_number_according_to_node(&info, &mut rt);

        // Some events might come in while we're waiting to grab the lock.
        // Replay them.
        while let Ok(event) = db_event_subscription.try_recv() {
            if inner.shutdown_receiver.has_changed().unwrap_or(true) {
                tracing::info!("The sequencer is shutting down. Exiting replay_batch");
                return Ok(data);
            }

            do_next_event(
                inner.seq_role,
                next_sequence_number_according_to_node,
                &mut executor,
                &tx_cache_writer,
                event,
                &mut data.batches_count,
                &mut data.transactions_count,
                &node_state_root,
                &mut data.batch_is_in_progress,
                &mut data.sequence_number_of_open_batch,
                &mut data.unprocessed_proofs,
            )
            .await?;
        }

        // The executor is now caught up. Swap it in
        inner.executor.replace_state(*executor).await;
        inner.sequence_number_of_open_batch = data.sequence_number_of_open_batch;
        Self::common_for_final_catchup_and_new_storage(&mut inner, info).await;

        drop(db_event_subscription);
        drop(inner);

        Ok(data)
    }

    async fn common_for_final_catchup_and_new_storage(
        inner: &mut InnerGuard<'_, S, Rt>,
        info: StateUpdateInfo<S::Storage>,
    ) {
        let node_sequence_number =
            get_next_sequence_number_according_to_node(&info, &mut Rt::default());

        if node_sequence_number > inner.next_unassigned_sequence_number {
            inner.next_unassigned_sequence_number = node_sequence_number;
        }

        inner.executor_rebase_height =
            StateCheckpoint::new(info.storage.clone(), &Rt::default().kernel(), None)
                .rollup_height_to_access();

        inner.is_ready = Ok(());
        inner.has_finished_startup = true;
        inner.latest_info = info;
        let checkpoint = inner
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache_and_ignoring_pinned_cache();
        inner
            .executor_events_sender
            .force_update_api_state(checkpoint)
            .await;
        inner
            .executor_events_sender
            .update_api_ledger_from_info(&inner.latest_info)
            .await;
    }

    async fn process_prune_sequencer_db(&mut self, reason: &'static str) {
        let start_prune = std::time::Instant::now();
        let mut inner = self.get_inner_with_timing(reason).await;
        if !inner.is_replica_role() {
            inner.trigger_batch_production_if_convenient().await;
        }
        inner.prune_sequencer_db().await;
        inner.start_replica_task_notifier.notify();
        drop(inner);

        let prune_duration = start_prune.elapsed();
        let metrics = PreferredSequencerPruneMetrics {
            duration_ms: prune_duration.as_millis() as u64,
        };
        sov_metrics::track_metrics(|t| {
            t.submit(metrics);
        });
    }

    async fn process_force_overwrite_state_for_recovery(
        &mut self,
        info: StateUpdateInfo<S::Storage>,
        reason: &'static str,
    ) {
        let mut inner = self.get_inner_with_timing(reason).await;

        // Since we're entering recovery, we don't re-use any of the uncommitted changes.
        // We don't need to populate the pinned cache because we'll replace the executor when we exit recovery before going back to normal operation.
        let recovery_executor = inner.new_executor_with_empty_uncommitted_changes(&info, None);

        inner
            .force_overwrite_state(info.clone(), recovery_executor)
            .await;
        inner
            .executor_events_sender
            .update_api_ledger_from_info(&info)
            .await;
    }

    async fn process_wait_for_node_resync(
        &mut self,
        info: StateUpdateInfo<S::Storage>,
        _distance: u64,
        reason: &'static str,
    ) {
        let mut inner = self.get_inner_with_timing(reason).await;
        let mut rt = Rt::default();
        inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
            target_da_height: info.sync_status.target_da_height(),
            synced_da_height: info.sync_status.synced_da_height(),
        });

        let node_sequence_number = get_next_sequence_number_according_to_node(&info, &mut rt);
        let our_sequence_number = inner.next_unassigned_sequence_number;

        if node_sequence_number > our_sequence_number {
            inner
                .overwrite_next_sequence_number_for_recovery(node_sequence_number)
                .await;
        }

        inner.latest_info = info.clone();
        // We update the API state, so users can query node state as it syncs.
        let checkpoint = StateCheckpoint::new(info.storage.clone(), &rt.kernel(), None);
        inner
            .executor_events_sender
            .update_state_for_recovery(checkpoint)
            .await;
        inner
            .executor_events_sender
            .update_api_ledger_from_info(&info)
            .await;
    }

    /// Closes the current batch
    #[cfg(feature = "test-utils")]
    async fn process_force_close_current_batch(
        &mut self,
        reason: &'static str,
        result_sender: oneshot::Sender<bool>,
    ) {
        let mut inner = self.get_inner_with_timing(reason).await;
        if !inner.executor.has_in_progress_batch() {
            let _ = result_sender.send(false); // If the receiver has dropped, we don't need to do anything about it.
            return;
        }
        inner.close_current_batch().await;
        let _ = result_sender.send(true);
    }

    async fn process_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: SerializedProofWithDetailsBytes,
        reason: &'static str,
    ) {
        let mut inner = self.get_inner_with_timing(reason).await;
        let sequence_number = inner.take_sequence_number_for_proof();
        let proof_bytes =
            proof_bytes(&data.0, sequence_number).expect("Serialization to vec is infallible");
        inner
            .process_proof(blob_id, proof_bytes, sequence_number)
            .await;
    }

    async fn process_trigger_batch_production(&mut self, reason: &'static str) {
        // We don't run force_overwrite_state() here.
        // This is mostly fine, mainly the API state will be out of date until we've
        // finished sending our batches.
        // Adding parallel state update handling is not worth the complexity right now.
        let mut inner = self.get_inner_with_timing(reason).await;
        inner.trigger_batch_production().await;
    }

    async fn process_accept_tx(
        &mut self,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        ip_and_credential: IpAndCredentialId<S::Address>,
        reason: &'static str,
    ) -> Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>> {
        let sequencing_data = self
            .runtime
            .sequencing_data_handler()
            .create_sequencing_data();
        let mut inner = self.get_inner_with_timing(reason).await;

        if inner.is_replica_role() {
            // The sequencer is running in replica mode and cannot accept transactions.
            return Err(AcceptTxError::ReplicaMode);
        }

        // If the sequencer had to give out 503s at any point during the time we were waiting for the lock, we need to return a 503 - otherwise
        // we've effectively jumped the line
        let new_tx_queue_id = inner.tx_queue_id.load(Ordering::Acquire);
        if new_tx_queue_id != original_tx_queue_id {
            tracing::debug!(%tx_hash, "Transaction was queued before downtime. Dropping.");
            return Err(AcceptTxError::SequencerOverloaded503);
        }

        inner
            .check_readiness(
                inner.seq_config.max_concurrent_blobs,
                inner.stop_at_rollup_height,
            )
            .await
            .map_err(AcceptTxError::NotFullySynced)?;

        if let Err(batch_creation_error) = inner
            .try_to_create_and_start_batch_if_none_in_progress(false)
            .await
        {
            // On all errors, we treat the sequencer as having had downtime and clear out the transaction queue.
            // Note that we'll increment the queue ID once per rejected tx. This is totally fine - we have 2**64 ids to play with
            // and atomic increments are very cheap relative to the cost of executing the tx
            inner.tx_queue_id.fetch_add(1, Ordering::AcqRel);

            return Err(AcceptTxError::BatchError {
                batch_creation_error,
                nb_of_concurrent_blob_submissions: inner.nb_of_concurrent_blob_submissions(),
            });
        };

        let token = inner
            .rate_limiter
            .allow(ip_and_credential.ip_addr, ip_and_credential.address)
            .map_err(|err| AcceptTxError::RateLimiter(err))?;

        let mut baked_tx = baked_tx;
        // Important: we read the sequencing data from the baked tx inside apply_tx_to_in_progress_batch (which is called from do_new_tx)
        // so this must not be moved without updating do_new_tx. See the comment in apply_tx_to_in_progress_batch for more details.
        baked_tx.set_sequencing_metadata(&sequencing_data);
        let (res, resource_used) = inner.do_new_tx(tx_hash, baked_tx).await;

        // Do not use `?` or return early here. We must always call `rate_limiter.update`
        // to ensure the limits are updated even for unsuccessful transactions.
        let res = res.map_err(AcceptTxError::NewTxError);
        inner.rate_limiter.update(token, resource_used);
        let (rx, remaining_slot_gas) = res?;

        inner.close_batch_if_nearly_full(remaining_slot_gas).await;

        Ok(rx)
    }

    async fn process_do_batch_start_replica(
        &mut self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let mut inner = self.get_inner_with_timing(reason).await;
        let seq_nr_of_next_blob_for_this_executor = inner.next_unassigned_sequence_number;
        let seq_nr_from_master = batch_from_master.sequence_number;

        debug!(
            % seq_nr_from_master,
            % seq_nr_of_next_blob_for_this_executor,
            "Entering process_do_batch_start_replica"
        );

        validate_db_data_from_replica(
            inner.has_finished_startup,
            &inner.is_ready,
            DbData::BatchStart(batch_from_master),
            seq_nr_of_next_blob_for_this_executor,
            seq_nr_from_master,
        )?;

        let batch_from_master =
            Self::ensure_replica_batch_start_visible_slot_matches(&mut inner, batch_from_master)?;

        Self::ensure_replica_batch_start_within_rebase_window(&mut inner, batch_from_master)?;

        inner
            .do_batch_start(
                batch_from_master.visible_slot_number_after_increase,
                batch_from_master.visible_slots_to_advance,
            )
            .await?;

        debug!(
            % seq_nr_from_master,
            % seq_nr_of_next_blob_for_this_executor,
            "Exiting process_do_batch_start_replica"
        );

        inner
            .start_replica_task_notifier
            .set_replica_processed_first_batch();

        Ok(())
    }

    async fn process_do_new_tx_replica(
        &mut self,
        seq_nr_from_master: u64,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let mut inner = self.get_inner_with_timing(reason).await;
        let db_data = DbData::Transaction(seq_nr_from_master, baked_tx.clone(), tx_hash);
        validate_db_data_from_replica_for_open_batch(
            inner.has_finished_startup,
            &inner.is_ready,
            db_data.clone(),
            inner.sequence_number_of_open_batch,
            inner.next_unassigned_sequence_number,
        )?;

        let (res, _) = inner.do_new_tx(tx_hash, baked_tx).await;
        let _ = res.map_err(ReplicaError::NewTx)?.0.await;

        Ok(())
    }

    async fn process_close_current_batch_replica(
        &mut self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let mut inner = self.get_inner_with_timing(reason).await;

        let db_data = DbData::BatchEnd(batch_from_master);
        let seq_nr_from_master = db_data.sequence_number();

        let seq_nr_of_current_blob_for_this_executor =
            validate_db_data_from_replica_for_open_batch(
                inner.has_finished_startup,
                &inner.is_ready,
                db_data.clone(),
                inner.sequence_number_of_open_batch,
                inner.next_unassigned_sequence_number,
            )?;

        debug!(
            % seq_nr_from_master,
            % seq_nr_of_current_blob_for_this_executor,
            "Entering process_close_current_batch_replica"
        );

        inner.close_current_batch().await;

        debug!(
            % seq_nr_from_master,
            % seq_nr_of_current_blob_for_this_executor,
            "Exiting process_close_current_batch_replica"
        );

        Ok(())
    }

    async fn process_new_proof_replica(
        &mut self,
        sequence_number_of_proof: u64,
        proof_bytes: PreferredProofDataBytes,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let mut inner = self.get_inner_with_timing(reason).await;

        let next_unassigned_sequence_number = inner.next_unassigned_sequence_number;
        debug!(
            % sequence_number_of_proof,
            % next_unassigned_sequence_number,
            "Entering process_new_proof_replica"
        );

        validate_db_data_from_replica(
            inner.has_finished_startup,
            &inner.is_ready,
            DbData::NewProof(sequence_number_of_proof, proof_bytes.clone()),
            next_unassigned_sequence_number,
            sequence_number_of_proof,
        )?;

        let assigned_sequence_number = inner.take_sequence_number_for_proof();
        assert_eq!(assigned_sequence_number, next_unassigned_sequence_number, "The sequence number for the proof should be the next unassigned sequence number. This is a bug, please report it.");
        inner
            .process_proof(new_blob_id(), proof_bytes, assigned_sequence_number)
            .await;

        debug!(
            % sequence_number_of_proof,
            % assigned_sequence_number,
            "Exiting process_new_proof_replica"
        );

        Ok(())
    }

    fn ensure_replica_batch_start_visible_slot_matches(
        inner: &mut InnerGuard<'_, S, Rt>,
        batch_from_master: BatchToStore,
    ) -> Result<BatchToStore, ReplicaError<S>> {
        let mut replica_vsn = inner.executor.checkpoint.current_visible_slot_number();
        let replica_expected =
            replica_vsn.advance(batch_from_master.visible_slots_to_advance.get().into());

        if replica_expected == batch_from_master.visible_slot_number_after_increase {
            return Ok(batch_from_master);
        }

        tracing::warn!(
            %replica_expected,
            master_expected = %batch_from_master.visible_slot_number_after_increase,
            "Replica VSN diverged from master. Entering sync mode and retrying."
        );

        let sync_details = SequencerNotReadyDetails::Syncing {
            target_da_height: inner.latest_info.sync_status.target_da_height(),
            synced_da_height: inner.latest_info.sync_status.synced_da_height(),
        };

        inner.is_ready = Err(sync_details.clone());

        Err(ReplicaError::NotReady(
            sync_details,
            Box::new(DbData::BatchStart(batch_from_master)),
        ))
    }

    fn ensure_replica_batch_start_within_rebase_window(
        inner: &mut InnerGuard<'_, S, Rt>,
        batch_from_master: BatchToStore,
    ) -> Result<(), ReplicaError<S>> {
        let state_root_delay_blocks: u64 =
            sov_modules_api::macros::config_value!("STATE_ROOT_DELAY_BLOCKS");
        let current_height = inner.executor.checkpoint.rollup_height_to_access();
        let rebase_height = inner.executor_rebase_height;

        if current_height.get().saturating_sub(rebase_height.get())
            <= state_root_delay_blocks.saturating_sub(1)
        {
            return Ok(());
        }

        let heights_since_rebase = current_height.get().saturating_sub(rebase_height.get());

        tracing::warn!(
            %current_height,
            %rebase_height,
            %heights_since_rebase,
            %state_root_delay_blocks,
            "Replica has accepted too many PG batches since the last executor rebase. Rejecting batch start until node replay catches up."
        );

        let sync_details = SequencerNotReadyDetails::Syncing {
            target_da_height: inner.latest_info.sync_status.target_da_height(),
            synced_da_height: inner.latest_info.sync_status.synced_da_height(),
        };

        inner.is_ready = Err(sync_details.clone());

        Err(ReplicaError::NotReady(
            sync_details,
            Box::new(DbData::BatchStart(batch_from_master)),
        ))
    }
}

fn validate_db_data_from_replica<S: Spec>(
    has_finished_startup: bool,
    is_ready: &Result<(), SequencerNotReadyDetails>,
    ret: DbData,
    seq_nr_for_this_executor: u64,
    seq_nr_from_master: u64,
) -> Result<(), ReplicaError<S>> {
    if !has_finished_startup {
        return Err(ReplicaError::NotReady(
            SequencerNotReadyDetails::Startup,
            ret.into(),
        ));
    }

    if let Err(err) = is_ready {
        tracing::debug!(?err, "Replica not ready");
        return Err(ReplicaError::NotReady(err.clone(), ret.into()));
    }

    if seq_nr_for_this_executor > seq_nr_from_master {
        tracing::debug!(%seq_nr_for_this_executor, %seq_nr_from_master, "Replica is ahead of master.");
        return Err(ReplicaError::Rejected(DBDataRejected::ExecutorAhead(
            seq_nr_for_this_executor,
        )));
    }

    if seq_nr_for_this_executor < seq_nr_from_master {
        tracing::debug!(%seq_nr_for_this_executor, %seq_nr_from_master, "Replica is behind master.");
        return Err(ReplicaError::Rejected(DBDataRejected::ExecutorBehind(ret)));
    }

    Ok(())
}

fn validate_db_data_from_replica_for_open_batch<S: Spec>(
    has_finished_startup: bool,
    is_ready: &Result<(), SequencerNotReadyDetails>,
    ret: DbData,
    sequence_number_of_open_batch: Option<u64>,
    next_unassigned_sequence_number: u64,
) -> Result<u64, ReplicaError<S>> {
    let seq_nr_from_master = ret.sequence_number();
    let seq_nr_for_this_executor = match sequence_number_of_open_batch {
        Some(seq_nr_for_this_executor) => seq_nr_for_this_executor,
        None => {
            if next_unassigned_sequence_number > seq_nr_from_master {
                tracing::debug!(
                    %next_unassigned_sequence_number,
                    %seq_nr_from_master,
                    "Replica is ahead of a stale event from master."
                );
                return Err(ReplicaError::Rejected(DBDataRejected::ExecutorAhead(
                    next_unassigned_sequence_number,
                )));
            }

            tracing::debug!(
                %seq_nr_from_master,
                %next_unassigned_sequence_number,
                "Replica is missing the matching batch start."
            );
            return Err(ReplicaError::Rejected(DBDataRejected::ExecutorBehind(ret)));
        }
    };

    validate_db_data_from_replica(
        has_finished_startup,
        is_ready,
        ret,
        seq_nr_for_this_executor,
        seq_nr_from_master,
    )?;

    Ok(seq_nr_for_this_executor)
}

#[derive(Debug)]
pub(crate) enum Flow {
    Break {
        pending_completed_proofs: Vec<PreferredProofToReplay>,
        in_progress_batch: Option<ReadBatch>,
        subscription: mpsc::Receiver<DbEvent>,
        fetch_in_progress_batch_time: Duration,
    },
    Continue {
        completed_blobs: Vec<PreferredBlobToReplay>,
    },
}

#[derive(Debug)]
pub(crate) struct FetchProofsAndCompletedBatches {
    pub(crate) metrics: PreferredSequencerFetchBatchesToReplayMetrics,
    pub(crate) flow: Flow,
}

fn completed_blobs_contain_batch(blobs: &[PreferredBlobToReplay]) -> bool {
    blobs
        .iter()
        .any(|blob| matches!(blob, PreferredBlobToReplay::Batch(_)))
}
