use sov_blob_storage::SequenceNumber;
use sov_modules_api::{Runtime, Spec};
use sov_rollup_interface::node::da::DaService;
use sov_state::{NativeStorage, Storage};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::metrics::PreferredSequencerUpdateStateMetrics;
use crate::preferred::transaction_subscriptions::TxResultWriter;
use crate::preferred::{
    get_next_sequence_number_according_to_node, DbEvent, FetchProofsAndCompletedBatches, Flow,
    PreferredBatchToReplay, PreferredBlobToReplay, PreferredProofToReplay, PreferredSequencer,
    ProcessFinalCatchupData, RollupBlockExecutor, StateUpdateInfo,
};
use crate::SequencerRole;
use tracing::error;

impl<S, Rt, Da> PreferredSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    /// Replays all outstanding batches on top of the given executor,
    /// starting from the last one processed by that executor.
    ///
    /// This function works by...
    /// - Getting the list of all *completed* batches from the database that haven't yet been played on this sequencer
    /// - Replaying each completed batch
    /// - Repeating until we've reached the in-progress batch. Then...
    ///
    /// Subscribe to DB events:
    /// - for each event, play it on the executor;
    /// - if we fell more than `config.sequencer_kind_config.db_event_channel_size` events behind, we block the sequencer. This should never happen with proper configuration, but we log a warning in case it does.
    #[tracing::instrument(skip_all, level = "debug", name = "update_state")]
    pub(super) async fn replay_soft_confirmations_on_top_of_node_state(
        &self,
        info: StateUpdateInfo<S::Storage>,
        timer_start: Instant,
        mut time_spent_fetching_batches: std::time::Duration, // The time already spent fetching batches to replay
        mut executor: Box<RollupBlockExecutor<S, Rt>>,
    ) -> anyhow::Result<()> {
        // On shutdown exit early. This prevents duplicate subscriptions to the DB events channel, which would cause spurious warnings.
        // Note that we only need to detect whether a previous `replay_soft_confirmations_on_top_of_node_state` was aborted due to shutdown
        // *while its subscription was active*, so a single check at the start is sufficient.
        if self.shutdown_receiver.has_changed().unwrap_or(true) {
            tracing::info!("The sequencer is shutting down. Exiting replay_soft_confirmations_on_top_of_node_state without completing replay.");
            return Ok(());
        }

        let mut batches_count = 0;
        let mut transactions_count = 0;
        let mut min_sequence_number_of_open_batch =
            get_next_sequence_number_according_to_node(&info, &mut Rt::default());
        // Total time to update the state for `replay_soft_confirmations_on_top_of_node_stat`e, including time spent in the `Message` channel.
        let mut total_message_processing_duration = std::time::Duration::ZERO;
        let tx_cache_writer = self.transaction_cache.write_handle();

        // Now that we're not locking on the sequencer state anymore, we can replay all the batches.

        let node_state_root = tracing::trace_span!("root_hash")
            .in_scope(|| info.storage.get_root_hash(info.slot_number))?;

        // Repeatedly fetch all completed batches from the database that haven't yet been played on this sequencer and replay them
        let mut unprocessed_proofs = BTreeMap::new();
        let (in_progress_batch, mut db_event_subscription, mut unprocessed_proofs) = loop {
            let (
                FetchProofsAndCompletedBatches {
                    metrics: fetch_batches_to_replay_metrics,
                    flow,
                },
                message_processing_duration,
            ) = self
                .synchronized_state_updator
                .fetch_proofs_and_completed_batches_msg(
                    min_sequence_number_of_open_batch,
                    "update_state::fetch_completed_batches_iteration",
                )
                .await
                .map_err(|e| e.into_state_update_error())?;

            total_message_processing_duration += message_processing_duration;

            let completed_blobs = match flow {
                Flow::Break {
                    pending_completed_proofs,
                    in_progress_batch,
                    subscription,
                    fetch_in_progress_batch_time,
                } => {
                    // Update metrics
                    {
                        time_spent_fetching_batches += fetch_batches_to_replay_metrics.duration;
                        time_spent_fetching_batches += fetch_in_progress_batch_time;
                        sov_metrics::track_metrics(|t| {
                            t.submit(fetch_batches_to_replay_metrics);
                        });
                    }

                    extend_pending_completed_proofs(
                        &mut unprocessed_proofs,
                        pending_completed_proofs,
                    );
                    break (in_progress_batch, subscription, unprocessed_proofs);
                }
                Flow::Continue { completed_blobs } => completed_blobs,
            };

            // Update metrics
            {
                time_spent_fetching_batches += fetch_batches_to_replay_metrics.duration;
                sov_metrics::track_metrics(|t| {
                    t.submit(fetch_batches_to_replay_metrics);
                });
            }

            // On each iteration, we'll get some proofs (say, sequence 3, 4, 6, and  8) and some completed batches (say, sequence 5 and 7).
            // We process the proofs when we start the next batch in sequence, and any proofs after the last batch are returned.
            // In our example we would...
            //  - Push proofs 3, 4 to unprocessed_proofs
            //  - Process batch 5, draining 3, 4 from unprocessed_proofs
            //  - Push proof 6
            //  - Process batch 7, draining 6 from unprocessed_proofs
            //  - Push batch 8. Since we don't have any completed batches after the last proof, unprocessed_proofs will have an entry when the loop exits.
            // Note that some "completed" proofs may have sequence numbers greater than the sequence number of the in-progress batch.
            // We have to be careful to ignore these proofs for now (they'll get processed when we start the *next*batch)
            for blob in completed_blobs {
                match blob {
                    PreferredBlobToReplay::Batch(batch) => {
                        batches_count += 1;
                        transactions_count += batch.batch.inner.data.len();
                        min_sequence_number_of_open_batch =
                            batch.batch.inner.sequence_number.saturating_add(1);
                        let proofs_to_replay = unprocessed_proofs
                            .extract_if(0.., |sequence_number, _| {
                                *sequence_number < batch.batch.inner.sequence_number
                            })
                            .map(|(_, proof)| proof)
                            .collect::<Vec<_>>();
                        executor
                            .replay_batch(&batch, proofs_to_replay, &node_state_root)
                            .await?;
                        if self.shutdown_receiver.has_changed().unwrap_or(true) {
                            tracing::info!("The sequencer is shutting down. Exiting replay_soft_confirmations_on_top_of_node_state.");
                            return Ok(());
                        }
                    }
                    PreferredBlobToReplay::Proof(proof) => {
                        unprocessed_proofs.insert(proof.sequence_number, proof);
                    }
                }
            }
        };

        // Now, we need to catch up by...
        // - Replaying the txs that are already present in the in-progress batch
        // - Replaying any db events that come in while we're catching up

        // Replay the in-progress batch if it exists.
        let mut batch_is_in_progress = false;
        let mut sequence_number_of_open_batch = None;
        if let Some(batch) = in_progress_batch {
            if let Err(err) = validate_batch_seq_nr_from_node(
                batch.sequence_number,
                min_sequence_number_of_open_batch,
                self.seq_role,
            ) {
                match err {
                    SequenceNumberMismatchError::SkipStateUpdate => return Ok(()),
                    SequenceNumberMismatchError::Other(err) => return Err(err),
                }
            }

            batches_count += 1;
            transactions_count += batch.txs.len();
            batch_is_in_progress = true;
            sequence_number_of_open_batch = Some(batch.sequence_number);
            // Only process proofs with sequence numbers less than the in-progress batch.
            let proofs_to_replay = unprocessed_proofs
                .extract_if(0.., |sequence_number, _| {
                    *sequence_number < batch.sequence_number
                })
                .map(|(_, proof)| proof)
                .collect::<Vec<_>>();
            let in_progress_batch = PreferredBatchToReplay {
                is_in_progress: true,
                visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
                batch: batch.into_with_cached_tx_hashes(),
            };

            executor
                .replay_batch(&in_progress_batch, proofs_to_replay, &node_state_root)
                .await?;
        }

        // Replay any db events that have come in while we're doing that catchup.
        // We don't require the channel to become completely empty because of the jitter that might introduce
        // Just get close, then lock the sequencer. This will keep p99 reasonable while hopefully
        // minimizing the risk of extremely long catchup periods in `update_state`.
        while db_event_subscription.len() > 1 {
            if self.shutdown_receiver.has_changed().unwrap_or(true) {
                tracing::info!("The sequencer is shutting down. Exiting replay_batch");
                return Ok(());
            }
            let event = db_event_subscription.try_recv().unwrap();
            if let Err(err) = do_next_event(
                self.seq_role,
                min_sequence_number_of_open_batch,
                &mut executor,
                &tx_cache_writer,
                event,
                &mut batches_count,
                &mut transactions_count,
                &node_state_root,
                &mut batch_is_in_progress,
                &mut sequence_number_of_open_batch,
                &mut unprocessed_proofs,
            )
            .await
            {
                match err {
                    SequenceNumberMismatchError::SkipStateUpdate => return Ok(()),
                    SequenceNumberMismatchError::Other(err) => return Err(err),
                }
            }
        }

        let (maybe_data, message_processing_duration) = self
            .synchronized_state_updator
            .final_catchup_msg(
                info,
                db_event_subscription,
                executor,
                node_state_root.clone(),
                ProcessFinalCatchupData {
                    batches_count,
                    transactions_count,
                    batch_is_in_progress,
                    sequence_number_of_open_batch,
                    unprocessed_proofs,
                },
                "update_state::do_final_catchup",
            )
            .await
            .map_err(|e| e.into_state_update_error())?;

        let data = match maybe_data {
            Ok(data) => data,
            Err(SequenceNumberMismatchError::SkipStateUpdate) => return Ok(()),
            Err(SequenceNumberMismatchError::Other(e)) => return Err(e),
        };

        total_message_processing_duration += message_processing_duration;

        let metrics = PreferredSequencerUpdateStateMetrics {
            duration: timer_start.elapsed(),
            total_message_processing_duration,
            batches_count: data.batches_count,
            transactions_count: data
                .transactions_count
                .try_into()
                .expect("transactions in a single batch cannot possibly exceed u64::MAX"),
            in_progress_batch: data.batch_is_in_progress,
            time_spent_fetching_batches,
        };

        sov_metrics::track_metrics(|t| {
            t.submit(metrics);
        });

        if !self.shutdown_receiver.has_changed().unwrap_or(true) {
            self.synchronized_state_updator
                .prune_sequencer_db_msg("update_state::prune_sequencer_db")
                .await
                .map_err(|e| e.into_state_update_error())?;
        }

        Ok(())
    }

    pub(super) async fn do_simple_state_update(
        &self,
        info: StateUpdateInfo<S::Storage>,
    ) -> anyhow::Result<()> {
        self.synchronized_state_updator
            .send_simple_state_update_msg(info)
            .await
            .map_err(|e| e.into_state_update_error())?;
        if !self.shutdown_receiver.has_changed().unwrap_or(true) {
            self.synchronized_state_updator
                .prune_sequencer_db_msg("update_state::prune_sequencer_db")
                .await
                .map_err(|e| e.into_state_update_error())?;
        }
        Ok(())
    }
}

fn extend_pending_completed_proofs(
    unprocessed_proofs: &mut BTreeMap<SequenceNumber, PreferredProofToReplay>,
    pending_completed_proofs: Vec<PreferredProofToReplay>,
) {
    for proof in pending_completed_proofs {
        unprocessed_proofs.insert(proof.sequence_number, proof);
    }
}

/// The sequencer number and node sequence number do not match.
pub enum SequenceNumberMismatchError {
    /// The sequence number mismatch can be resolved by skipping the `state_update`.
    SkipStateUpdate,
    /// The error cannot be resolved locally and must be propagated to the caller.
    Other(anyhow::Error),
}

fn validate_batch_seq_nr_from_node(
    seq_nr_of_in_progress_batch: u64,
    next_batch_sequence_number_according_to_node: u64,
    seq_role: SequencerRole,
) -> Result<(), SequenceNumberMismatchError> {
    if seq_nr_of_in_progress_batch < next_batch_sequence_number_according_to_node {
        match seq_role {
            SequencerRole::PgSyncReplica => {
                // If this occurs on replicas, we log the error and skip `update_state` for the batch received from the node.
                // If the database slowdown is temporary, the issue will be resolved when the next `update_state` call succeeds.
                // If the situation persists, the replica will eventually enter sync mode in that case that the database setup needs to be examined.
                error!(seq_nr_of_in_progress_batch, next_batch_sequence_number_according_to_node, "The replica has an in-progress batch whose sequence number is lower than the next_sequence_number expected by the node. 
                    This indicate that Postgres notifications are delayed. In this case, the update from the node is ignored. 
                    If this error occurs repeatedly, investigate the database stack in the deployment.");
                return Err(SequenceNumberMismatchError::SkipStateUpdate);
            }
            SequencerRole::BatchProducer | SequencerRole::DaOnlyReplica => {
                // For roles other than PgSyncReplica, we should never observe in-progress batches with sequence numbers lower than what the node expects (DaOnlyReplica doesn't create batches).
                let err = anyhow::anyhow!(
                    "sequencer_role: {seq_role:?},
                    seq_nr_of_in_progress_batch: {seq_nr_of_in_progress_batch}, 
                    next_sequence_number_according_to_node: {next_batch_sequence_number_according_to_node},
                    The sequencer has an in-progress batch whose sequence number is lower than the next_sequence_number expected by the node.
                    This is a bug, please report it."
                );

                return Err(SequenceNumberMismatchError::Other(err));
            }
        }
    }
    Ok(())
}

/// Replay an event on the executor.
#[tracing::instrument(skip_all, level = "warn", name = "update_state::do_next_event")]
pub(crate) async fn do_next_event<S: Spec, Rt: Runtime<S>>(
    seq_role: SequencerRole,
    next_sequence_number_according_to_node: u64,
    executor: &mut RollupBlockExecutor<S, Rt>,
    tx_cache_writer: &TxResultWriter<S, Rt>,
    event: DbEvent,
    batches_count: &mut u64,
    transactions_count: &mut usize,
    node_state_root: &<S::Storage as Storage>::Root,
    batch_is_in_progress: &mut bool,
    sequence_number_of_open_batch: &mut Option<SequenceNumber>,
    unprocessed_proofs: &mut BTreeMap<SequenceNumber, PreferredProofToReplay>,
) -> Result<(), SequenceNumberMismatchError> {
    match event {
        DbEvent::TxAccepted(tx, hash) => {
            executor.replay_tx(hash, tx).await;
            *transactions_count += 1;
            *batch_is_in_progress = true;
        }
        DbEvent::BatchClosed(sequence_number) => {
            validate_batch_seq_nr_from_node(
                sequence_number,
                next_sequence_number_according_to_node,
                seq_role,
            )?;

            tracing::trace!("Done replaying txs");
            let forced_txs = executor.end_rollup_block().await;
            for tx in forced_txs {
                tx_cache_writer.insert(tx).await;
            }
            *batch_is_in_progress = false;
            *sequence_number_of_open_batch = None;
        }
        DbEvent::BatchStarted {
            sequence_number,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        } => {
            validate_batch_seq_nr_from_node(
                sequence_number,
                next_sequence_number_according_to_node,
                seq_role,
            )?;

            *batches_count += 1;
            // Replay all the proofs with sequence numbers less than the batch sequence number.
            let proofs_to_replay = unprocessed_proofs
                .extract_if(0.., |proof_sequence_number, _| {
                    *proof_sequence_number < sequence_number
                })
                .map(|(_, proof)| proof)
                .collect::<Vec<_>>();
            executor
                .start_rollup_block_for_replay(
                    visible_slot_number_after_increase,
                    visible_slots_to_advance,
                    node_state_root,
                    0,
                    proofs_to_replay,
                )
                .await;

            *batch_is_in_progress = true;
            *sequence_number_of_open_batch = Some(sequence_number);
        }
        DbEvent::ProofBlobAccepted {
            sequence_number,
            proof_bytes,
        } => {
            // Note that we also don't change the state of the batch_is_in_progress flag here.
            tracing::trace!("Proof blob accepted");

            unprocessed_proofs.insert(
                sequence_number,
                PreferredProofToReplay {
                    sequence_number,
                    data: proof_bytes,
                },
            );
        }
    }
    Ok(())
}
