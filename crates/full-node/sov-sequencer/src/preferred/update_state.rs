use std::time::Instant;

use sov_modules_api::{Runtime, Spec};
use sov_rollup_interface::node::da::DaService;
use sov_state::{NativeStorage, Storage};

use crate::metrics::PreferredSequencerUpdateStateMetrics;
use crate::preferred::{
    get_next_sequence_number_according_to_node, DbEvent, FetchBatches, Flow,
    PreferredBatchToReplay, PreferredSequencer, ProcessFinalCatchupData, RollupBlockExecutor,
    RollupBlockExecutorConfig, StateUpdateInfo,
};

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
        is_startup_or_resync: bool,
        mut time_spent_fetching_batches: std::time::Duration, // The time already spent fetching batches to replay
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
        let mut next_sequence_number =
            get_next_sequence_number_according_to_node(&info, &mut Rt::default());
        // Total time to update the state for `replay_soft_confirmations_on_top_of_node_stat`e, including time spent in the `Message` channel.
        let mut total_message_processing_duration = std::time::Duration::ZERO;

        // During startup, we need to repopulate the transaction cache with any transactions from the soft-confirmed batches
        // Outside of this edge case, we don't want replay to affect the transaction cache, so we don't pass a writer.
        let startup_transaction_cache_writer = if is_startup_or_resync {
            Some(self.transaction_cache.write_handle())
        } else {
            None
        };

        let rollup_exec_config = RollupBlockExecutorConfig {
            config: self.config.clone(),
            da_address: self.da_address.clone(),
            shutdown_notifier: self.block_executors_shutdown_notifier.clone(),
            state_root_request_sender: self.state_root_compute_task.request_sender.clone(),
            shutdown_receiver: self.shutdown_receiver.clone(),
            shutdown_sender: self.shutdown_sender.clone(),
            startup_transaction_cache_writer,
        };

        // Now that we're not locking on the sequencer state anymore, we can replay all the batches.
        let mut executor = RollupBlockExecutor::<_, Rt>::new(&info, rollup_exec_config);

        let node_state_root = tracing::trace_span!("root_hash")
            .in_scope(|| info.storage.get_root_hash(info.slot_number))?;

        if is_startup_or_resync {
            self.transaction_cache
                .clean_and_overwrite_next_tx_number(info.next_tx_number)
                .await;
        }

        // Repeatedly fetch all completed batches from the database that haven't yet been played on this sequencer and replay them
        let (in_progress_batch, mut db_event_subscription) = loop {
            let (
                FetchBatches {
                    metrics: fetch_batches_to_replay_metrics,
                    flow,
                },
                message_processing_duration,
            ) = self
                .synchronized_state_updator
                .fetch_completed_batches_msg(
                    next_sequence_number,
                    "update_state::fetch_completed_batches_iteration",
                )
                .await;

            total_message_processing_duration += message_processing_duration;

            let completed_batches = match flow {
                Flow::Break {
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

                    break (in_progress_batch, subscription);
                }
                Flow::Continue { completed_batches } => completed_batches,
            };

            // Update metrics
            {
                time_spent_fetching_batches += fetch_batches_to_replay_metrics.duration;
                sov_metrics::track_metrics(|t| {
                    t.submit(fetch_batches_to_replay_metrics);
                });
            }

            for batch in completed_batches {
                batches_count += 1;
                transactions_count += batch.batch.inner.data.len();
                next_sequence_number = batch.batch.inner.sequence_number.saturating_add(1);
                executor.replay_batch(&batch, &node_state_root).await?;
                if self.shutdown_receiver.has_changed().unwrap_or(true) {
                    tracing::info!("The sequencer is shutting down. Exiting replay_soft_confirmations_on_top_of_node_state.");
                    return Ok(());
                }
            }
        };

        // Now, we need to catch up by...
        // - Replaying the txs that are already present in the in-progress batch
        // - Replaying any db events that come in while we're catching up

        // Replay the in-progress batch if it exists.
        let mut batch_is_in_progress = false;
        if let Some(batch) = in_progress_batch {
            batches_count += 1;
            transactions_count += batch.txs.len();
            batch_is_in_progress = true;
            let in_progress_batch = PreferredBatchToReplay {
                is_in_progress: true,
                visible_slot_number_after_increase: batch.visible_slot_number_after_increase,
                batch: batch.into_with_cached_tx_hashes(),
            };

            executor
                .replay_batch(&in_progress_batch, &node_state_root)
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
            do_next_event(
                &mut executor,
                event,
                &mut batches_count,
                &mut transactions_count,
                &node_state_root,
                &mut batch_is_in_progress,
            )
            .await?;
        }

        let (maybe_data, message_processing_duration) = self
            .synchronized_state_updator
            .final_catchup_msg(
                self.api_ledger_db.clone(),
                self.transaction_cache.write_handle(),
                info,
                db_event_subscription,
                executor,
                node_state_root.clone(),
                ProcessFinalCatchupData {
                    batches_count,
                    transactions_count,
                    batch_is_in_progress,
                },
                "update_state::do_final_catchup",
            )
            .await;

        let data = maybe_data?;

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
                .await;
        }

        Ok(())
    }
}

/// Replay an event on the executor.
#[tracing::instrument(skip_all, level = "warn", name = "update_state::do_next_event")]
pub(crate) async fn do_next_event<S: Spec, Rt: Runtime<S>>(
    executor: &mut RollupBlockExecutor<S, Rt>,
    event: DbEvent,
    batches_count: &mut u64,
    transactions_count: &mut usize,
    node_state_root: &<S::Storage as Storage>::Root,
    batch_is_in_progress: &mut bool,
) -> anyhow::Result<()> {
    match event {
        DbEvent::TxAccepted(tx, hash) => {
            executor.replay_tx(hash, &tx).await;
            *transactions_count += 1;
            *batch_is_in_progress = true;
        }
        DbEvent::BatchClosed(_) => {
            tracing::trace!("Done replaying txs");
            executor.end_rollup_block().await;
            *batch_is_in_progress = false;
        }
        DbEvent::BatchStarted {
            sequence_number: _,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        } => {
            *batches_count += 1;
            executor
                .start_rollup_block_for_replay(
                    visible_slot_number_after_increase,
                    visible_slots_to_advance,
                    node_state_root,
                    0,
                )
                .await;

            *batch_is_in_progress = true;
        }
        DbEvent::ProofBlobAccepted(_) => {
            // We don't do anything with proofs yet.
            // Note that we also don't change the state of the batch_is_in_progress flag here.
            tracing::trace!("Proof blob accepted");
        }
    }
    Ok(())
}
