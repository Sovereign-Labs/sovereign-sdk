use anyhow::Context as _;

use crate::metrics::{
    track_sequence_number, track_sequence_number_delta, PreferredSequencerChannelMetrics,
    PreferredSequencerChannelMetricsBatch,
};
use crate::preferred::block_executor::{
    AcceptedTxWithBudgetInfo, RollupBlockExecutor, RollupBlockExecutorError,
};
use crate::preferred::block_executor::{RollupBlockExecutorErrorWithBudget, StartBlockData};
use crate::preferred::cache_warm_up_executor::{CacheWarmUpExecutor, StartBlockNotification};
use crate::preferred::db::{latest_finalized_sequence_number, SequencerRole};
use crate::preferred::executor_events::ExecutorEventsSender;
use crate::preferred::rate_limiter::ResourceUsed;
use crate::preferred::rate_limiter::SovRateLimiter;
use crate::preferred::sync_sequencer_state::EventReceiverStartNotifier;
use crate::preferred::AcceptedTx;
use crate::preferred::BatchSizeTracker;
use crate::preferred::RollupBlockExecutorConfig;
use crate::preferred::{comfortable_gas_limit_for_height, PreferredBlobToReplay};
use crate::preferred::{
    current_visible_slot_number_according_to_node, get_next_sequence_number_according_to_node,
    is_lagging_less_than_ideal_amount, next_visible_slot_number_increase, BatchCreationError,
    Confirmation, PreferredSequencerConfig, PreferredSequencerFetchBatchesToReplayMetrics,
    TxResultWriter,
};
use crate::{
    PreferredProofDataBytes, SequencerConfig, SequencerNotReadyDetails, SlotNumber, TxHash,
};
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::Gas;
use sov_modules_api::{
    FullyBakedTx, GasArray, GasSpec, Runtime, Spec, StateCheckpoint, VersionReader,
    VisibleSlotNumber,
};
use sov_rollup_full_node_interface::StateUpdateInfo;
use sov_rollup_interface::stf::BlobSenderStatus;
use sov_state::{NativeStorage, Storage};
use std::num::NonZero;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
use tracing::{debug, info, warn};

/// These two constants are used to calculate the comfortable batch size limit.
/// Currently, this is 99% of the hard limit. After the comfortable limit is reached,
/// the sequencer will close and publish the current batch.
const COMFORTABLE_SIZE_LIMIT_MULTIPLIER: u64 = 99;
const COMFORTABLE_SIZE_LIMIT_DIVISOR: u64 = 100;

const COMFORTABLE_IN_FLIGHT_BLOBS: usize = 5;

const METRICS_BATCH_SIZE: usize = 32;
/// The constant for the I part of the PID controller for the batch size limit.
/// The larger this constant, the faster the bias will evolve, but the more likely we are to oscillate.
const BATCH_SIZE_LIMIT_BIAS_K_I: f64 = 0.1;
const EXECUTION_TIME_LIMIT_BIAS_POSITIVE_ERROR_K_I: f64 = 0.02; // Positive errors are slow to accumulate, because over-accepting txs early is not easily fixable
const EXECUTION_TIME_LIMIT_BIAS_NEGATIVE_ERROR_K_I: f64 = 0.2; // Negative errors accumulate faster, because rejecting txs too aggressively is easy to fix by accepting more later.

#[derive(Debug)]
pub(crate) enum DoNewTxError<S: Spec> {
    TxTooBig {
        current_batch_size: usize,
        max_batch_size: usize,
        tx_len: usize,
    },
    ExecutorError(RollupBlockExecutorError<S>),
    Shutdown,
}

/// A inner sequencer struct containing state that requires synchronized access.
/// This struct accepts/rejects transactions, then hands them to the side effects task
/// to be persisted.
pub(crate) struct Inner<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub(crate) seq_role: SequencerRole,
    pub(crate) seq_config: SequencerConfig<S::Address, PreferredSequencerConfig<S::Address>>,
    pub(crate) max_concurrent_proof_blobs: usize,
    pub(crate) shutdown_receiver: watch::Receiver<()>,
    pub(crate) shutdown_sender: watch::Sender<()>,

    pub(crate) executor: RollupBlockExecutor<S, Rt>,
    /// The rollup height of the latest node checkpoint applied to this executor's storage.
    pub(crate) executor_rebase_height: RollupHeight,
    pub(crate) latest_info: StateUpdateInfo<S::Storage>,
    pub(crate) batch_execution_time_limit_micros: u64,
    pub(crate) batch_size_tracker: BatchSizeTracker,
    pub(crate) is_ready: Result<(), SequencerNotReadyDetails>,
    /// Counts batch blobs only. Gates batch production so that proofs in flight
    /// cannot block new batches from being created (which is the only path
    /// that drains queued proofs via `proofs_for_replay`).
    pub(crate) in_flight_batch_blobs: Arc<AtomicUsize>,
    /// Counts proof blobs only. Used to gate new proof submissions when too
    /// many proofs are already in flight.
    pub(crate) in_flight_proof_blobs: Arc<AtomicUsize>,
    pub(crate) executor_events_sender: ExecutorEventsSender<S, Rt>,
    // We track two sequence numbers: the sequence number of the current open batch, and the next unassigned sequence number.
    // This is because we might need to assign a sequence number to some proofs while a batch is in progress,
    // and we don't want to forget what we've assigned.
    pub(crate) sequence_number_of_open_batch: Option<SequenceNumber>,
    pub(crate) next_unassigned_sequence_number: SequenceNumber,
    /// A boolean that indicates whether the sequencer has finished its startup phase.
    /// We need this rather than relying on `SequencerNotReadyDetails::Startup` because that state
    /// can be overwritten when the node is resyncing.
    pub(crate) has_finished_startup: bool,
    pub(crate) metrics: Vec<PreferredSequencerChannelMetrics>,
    // Shared between sequencer and Inner.
    pub(crate) tx_queue_id: Arc<AtomicU64>,
    pub(crate) stop_at_rollup_height: Option<RollupHeight>,
    pub(crate) rollup_exec_config: RollupBlockExecutorConfig<S>,
    pub(crate) tx_cache_writer: TxResultWriter<S, Rt>,
    pub(crate) cache_warm_up_executor: CacheWarmUpExecutor<S>,
    pub(crate) start_replica_task_notifier: EventReceiverStartNotifier,
    pub(crate) rate_limiter: SovRateLimiter<S>,
    pub(crate) pi_controller: PIController,
}

/// A PI controller for adjusting the batch size and execution time limit based on the current load.
/// The PI controller allows us to smoothly vary our tx rejection rate as the batch fills up, rather than
/// switching suddenly from "accept all" to "accept none".
///
/// At most every 10 millis we "tick" the PI controller, updating the state with the latest data about...
///  - The rate at which new transactions are being offered
///  - The average time/bytes to accept a tx
///  - The remaining available size/time limits.
///  - How much longer we expect the current batch to be open (based on estimated block times)
///
/// Based on that data, we set probabilities for accepting or rejecting new transactions. For example,
/// suppose that we're 1.5 seconds in to a 3 second block time, and we've accepted 4 MB of our 6MB limit. Then the probability of accepting a new
/// tx will drop to keep the batch size under control. Note that we compute probabilities for both execution time and batch size,
/// and then we take the max rejection probability across those two dimensions.
pub struct PIController {
    pub(crate) batch_start_time: std::time::Instant, // The time when the current batch was opened
    pub(crate) approximate_block_time: std::time::Duration,
    // The bias term for the batch size limiter (this lets us correct if we're repeatedly over or undershooting the target batch size)
    // Expressed in bytes per second. (I.e. if the `P` term of our controller says to accept 1000 bytes per second, and this bias is 40, then we will try to accept 1040 bytes per second.)
    // Can be negative
    pub(crate) size_limit_bias: f64,
    // The bias term for the execution time limiter (this lets us correct if we're repeatedly over or undershooting the target execution time)
    pub(crate) execution_time_limit_bias: f64,
    // The average time to execute a tx in microseconds. Computed as a EWMA over all txs since startup.
    pub(crate) estimated_tx_execution_time_micros: f64,
    // The last time we ticked the PI controller.
    pub(crate) last_tick_time: std::time::Instant,
    pub(crate) bytes_offered_since_last_tick: u64, // How many bytes worth of tx data we would have accepted given 100% acceptance rate
    pub(crate) bytes_offered_per_second_ewma: f64, // The weighted average of bytes offered per second. EWMA = Exponentially Weighted Moving Average
    pub(crate) current_tx_accept_rate_bytes_per_second: f64,
    pub(crate) execution_time_offered_since_last_tick: f64,
    pub(crate) execution_time_offered_per_second_ewma: f64,
    // The current rate at which to accept txs based on execution time, given in micros per second.
    // For example, if we're consuming our execution time budget just a little too fast, this will drop to something like 950000 micros per second.
    pub(crate) current_tx_accept_rate_execution_time_micros_per_second: f64,
    // An error correction term which rises when we accept a tx that had some non-zero chance of rejection.
    // For example, as we accept txs with a 20% chance of rejection, this will rise to .2, .4, .6, etc. Once it hits 1.0,
    // we deterministically reject the next tx and reset the debt to 0.
    pub(crate) load_shed_rejection_debt: f64,
}

#[allow(clippy::float_arithmetic)]
impl PIController {
    pub(crate) fn new(
        approximate_block_time: std::time::Duration,
        max_batch_size: usize,
        batch_execution_time_limit_micros: u64,
    ) -> Self {
        assert_ne!(
            approximate_block_time,
            std::time::Duration::ZERO,
            "Approximate block time must be non-zero"
        );
        Self {
            approximate_block_time,
            batch_start_time: std::time::Instant::now(),
            size_limit_bias: 0.0,
            execution_time_limit_bias: 0.0,
            estimated_tx_execution_time_micros: 0.0,
            last_tick_time: std::time::Instant::now(),
            bytes_offered_since_last_tick: 0,
            bytes_offered_per_second_ewma: 0.0,
            current_tx_accept_rate_bytes_per_second: max_batch_size as f64
                / approximate_block_time.as_secs_f64(),
            execution_time_offered_since_last_tick: 0.0,
            execution_time_offered_per_second_ewma: 0.0,
            current_tx_accept_rate_execution_time_micros_per_second:
                batch_execution_time_limit_micros as f64 / approximate_block_time.as_secs_f64(),
            load_shed_rejection_debt: 0.0,
        }
    }
}

// We submit metrics when this guard is dropped.
pub(crate) struct InnerGuard<'a, S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    inner: &'a mut Inner<S, Rt>,
    reason: &'static str,
    start_time: std::time::Instant,
    channel_size: u32,
}

impl<'a, S, Rt> InnerGuard<'a, S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    /// Create a new inner guard.
    pub fn new(inner: &'a mut Inner<S, Rt>, reason: &'static str, channel_size: u32) -> Self {
        Self {
            inner,
            reason,
            start_time: std::time::Instant::now(),
            channel_size,
        }
    }
}

impl<S, Rt> Deref for InnerGuard<'_, S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    type Target = Inner<S, Rt>;
    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl<S, Rt> std::ops::DerefMut for InnerGuard<'_, S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
    }
}

impl<S, Rt> Drop for InnerGuard<'_, S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    fn drop(&mut self) {
        self.inner.metrics.push(PreferredSequencerChannelMetrics {
            duration: self.start_time.elapsed(),
            reason: self.reason,
            channel_size: self.channel_size,
        });
        if self.inner.metrics.len() >= METRICS_BATCH_SIZE {
            sov_metrics::track_metrics(|t| {
                t.submit(PreferredSequencerChannelMetricsBatch {
                    metrics: std::mem::replace(
                        &mut self.inner.metrics,
                        Vec::with_capacity(METRICS_BATCH_SIZE),
                    ),
                });
            });
        }
    }
}

impl<S, Rt> Inner<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub(crate) fn nb_of_concurrent_batch_blob_submissions(&self) -> usize {
        self.in_flight_batch_blobs.load(Ordering::Relaxed)
    }

    pub(crate) async fn overwrite_next_sequence_number_for_recovery(
        &mut self,
        sequence_number: SequenceNumber,
    ) {
        info!(%sequence_number, "Overwriting next sequence number");
        self.next_unassigned_sequence_number = sequence_number;
        track_sequence_number(self.next_unassigned_sequence_number);
    }

    /// Assign a sequence number to the current open batch.
    pub(crate) fn assign_sequence_number_to_batch(&mut self) -> SequenceNumber {
        let sequence_number = self.take_sequence_number_internal();
        self.sequence_number_of_open_batch = Some(sequence_number);
        sequence_number
    }

    /// Take a sequence number for a proof blob.
    pub(crate) fn take_sequence_number_for_proof(&mut self) -> SequenceNumber {
        self.take_sequence_number_internal()
    }

    fn take_sequence_number_internal(&mut self) -> SequenceNumber {
        let sequence_number = self.next_unassigned_sequence_number;
        self.next_unassigned_sequence_number = self
            .next_unassigned_sequence_number
            .checked_add(1)
            .expect("Sequence number overflow; this should be unreachable for a few billion years");
        track_sequence_number(self.next_unassigned_sequence_number);
        sequence_number
    }

    pub(crate) async fn prune_sequencer_db(&mut self) {
        let next_sequence_number = self.next_unassigned_sequence_number;
        let latest_state_info = &self.latest_info;
        let mut runtime = Rt::default();
        let next_sequence_number_according_to_node =
            get_next_sequence_number_according_to_node(latest_state_info, &mut runtime);

        let delta = (next_sequence_number as i64) - (next_sequence_number_according_to_node as i64);
        track_sequence_number_delta(delta);

        match latest_finalized_sequence_number(latest_state_info, &mut runtime) {
            Some(num) => {
                self.executor_events_sender.prune(num).await;
            }
            None => {
                // Nothing to prune because there's no sequence number history.
            }
        }
    }

    pub(crate) async fn force_overwrite_state(
        &mut self,
        info: StateUpdateInfo<S::Storage>,
        new_executor: RollupBlockExecutor<S, Rt>,
    ) {
        tracing::trace!(?info, "Overwriting preferred sequencer internal state");

        // Replace known info
        self.latest_info = info.clone();
        self.executor_rebase_height = new_executor.checkpoint.rollup_height_to_access();

        // Replace executor state
        self.executor.replace_state(new_executor).await;

        // Replace API state
        let mut rt = Rt::default();
        let checkpoint = StateCheckpoint::new(info.storage.clone(), &rt.kernel());
        self.executor_events_sender
            .force_update_api_state(checkpoint)
            .await;
    }

    pub(crate) async fn trigger_recovery(&mut self, info: &StateUpdateInfo<S::Storage>) {
        let recovery_strategy = self
            .seq_config
            .sequencer_kind_config
            .recovery_strategy
            .clone();

        self.is_ready = Err(SequencerNotReadyDetails::PreferredSequencerRecovering);
        let next_sequence_number_according_to_node =
            get_next_sequence_number_according_to_node(info, &mut Rt::default());

        self.executor_events_sender
            .trigger_recovery(next_sequence_number_according_to_node, recovery_strategy)
            .await;

        // Creates a new executor for recovery. This must *not* be called to create executors
        // under other circumstances, since it causes side effects on the transaction cache.

        // Since we're entering recovery, we don't re-use any of the uncommitted changes.
        let recovery_executor = self.new_executor_with_empty_uncommitted_changes(info);

        self.force_overwrite_state(info.clone(), recovery_executor)
            .await;

        info!(?info, current_visible_slot_number = %current_visible_slot_number_according_to_node::<S,Rt>(info), "Beginning sequencer recovery");
    }

    #[tracing::instrument(skip_all, level = "trace")]
    pub(crate) fn proofs_and_completed_batches_for_replay(
        &self,
        sequence_number: SequenceNumber,
        include_in_progress_batch: bool,
    ) -> (
        Vec<PreferredBlobToReplay>,
        PreferredSequencerFetchBatchesToReplayMetrics,
    )
    where
        S: Spec,
        Rt: Runtime<S>,
    {
        let start = std::time::Instant::now();
        let result = self
            .executor_events_sender
            .fetch_proofs_and_completed_batches_by_sequence(
                sequence_number,
                include_in_progress_batch,
            );
        let duration = start.elapsed();
        let metrics = PreferredSequencerFetchBatchesToReplayMetrics {
            duration,
            num_batches: result.len() as u64,
            num_transactions: result.iter().map(|b| b.num_txs()).sum(),
        };
        (result, metrics)
    }

    /// Closes the current batch if it is nearly full (by gas limit) or has reached the target batch execution time.
    pub(crate) async fn close_batch_if_nearly_full(
        &mut self,
        remaining_slot_gas: <S as GasSpec>::Gas,
    ) {
        let rollup_height = self.executor.checkpoint.rollup_height_to_access();
        // Check if we're close to the gas limit and close the batch if we are.
        // We want to close when gas used is at least 95% of the initial gas limit.
        let initial_gas_limit = <S as GasSpec>::gas_limit_for_height(rollup_height);
        let comfortable_gas_limit = comfortable_gas_limit_for_height::<S>(rollup_height);

        let gas_used = initial_gas_limit
            .checked_sub(remaining_slot_gas)
            .expect("remaining_lot_gas is always smaller than initial_gas_limit");

        let close_to_gas_limit = comfortable_gas_limit.dim_is_less_or_eq(gas_used);
        if close_to_gas_limit {
            tracing::debug!(%comfortable_gas_limit, %gas_used, "Closing and publishing current batch because we're close to the gas limit");
            self.close_current_batch().await;
        }

        let current_batch_execution_time_micros =
            self.batch_size_tracker.batch_execution_time_micros;

        if current_batch_execution_time_micros > self.batch_execution_time_limit_micros {
            tracing::debug!(%self.batch_execution_time_limit_micros, %current_batch_execution_time_micros, "Closing and publishing current batch because we've reached the batch execution time cap");
            self.close_current_batch().await;
        } else {
            tracing::trace!(%self.batch_execution_time_limit_micros, %current_batch_execution_time_micros, "Batch execution time is within comfortable range, not closing batch");
        }

        let comfortable_size_limit = (self.batch_size_tracker.max_batch_size as u64)
            .checked_div(COMFORTABLE_SIZE_LIMIT_DIVISOR)
            .and_then(|x| x.checked_mul(COMFORTABLE_SIZE_LIMIT_MULTIPLIER))
            .unwrap_or_else(|| {
                panic!(
                    "Cannot overflow after dividing by {COMFORTABLE_SIZE_LIMIT_DIVISOR} and multiplying by {COMFORTABLE_SIZE_LIMIT_MULTIPLIER}",
                )
            });
        if (self.batch_size_tracker.current_batch_size as u64) > comfortable_size_limit {
            tracing::debug!(%comfortable_size_limit, current_batch_size = %self.batch_size_tracker.current_batch_size, "Closing and publishing current batch because we're close to the size limit");
            self.close_current_batch().await;
        } else {
            tracing::trace!(%comfortable_size_limit, current_batch_size = %self.batch_size_tracker.current_batch_size, "Batch size is within comfortable range, not closing batch");
        }
    }

    #[tracing::instrument(skip_all, level = "trace")]
    pub(crate) async fn trigger_batch_production_if_convenient(&mut self) {
        if !self.seq_config.automatic_batch_production {
            warn!("Skipping batch production due to settings");
            return;
        }

        // If we're lagging less than the ideal amount, it's not convenient to create a new batch so return early
        if is_lagging_less_than_ideal_amount(
            self.executor.checkpoint.current_visible_slot_number(),
            self.latest_info.latest_finalized_slot_number,
            self.seq_config
                .sequencer_kind_config
                .ideal_lag_behind_finalized_slot,
        ) {
            tracing::trace!(
                "Skipping batch production due to lagging less than ideal slot number difference"
            );
            return;
        }

        let in_flight_batch_blobs = self.nb_of_concurrent_batch_blob_submissions();
        if in_flight_batch_blobs >= COMFORTABLE_IN_FLIGHT_BLOBS {
            tracing::trace!(
                current_in_flight = %in_flight_batch_blobs,
                max_comfortable = %COMFORTABLE_IN_FLIGHT_BLOBS,
                "Skipping batch production due too many in flight batch blobs");
            return;
        }

        self.trigger_batch_production().await;
    }

    pub(crate) async fn trigger_batch_production(&mut self) {
        if !self.seq_config.automatic_batch_production {
            tracing::error!("Producing batch even though automatic batch production is disabled. This is probably a test bug");
            #[cfg(debug_assertions)]
            panic!("Producing batch even though automatic batch production is disabled. This is probably a test bug");
        }

        if let Err(e) = self
            .try_to_create_and_start_batch_if_none_in_progress(true)
            .await
        {
            tracing::debug!(
                error = %e,
                "Unable to start new batch after successful state update."
            );
        }

        // We were unable to open a new batch (likely due to a lack of finalized
        // slots), so we're done.
        if !self.executor.has_in_progress_batch() {
            return;
        }

        // If the node is shutting down, we may not be able to terminate the batch. In that case, just return early.
        if self.shutdown_receiver.has_changed().unwrap_or(true) {
            info!("The sequencer is shutting down. Exiting trigger_batch_production.");
            return;
        }

        self.close_current_batch().await;
    }

    pub(crate) async fn check_readiness(
        &self,
        max_concurrent_batch_blobs: usize,
        height_to_stop_at: Option<RollupHeight>,
    ) -> Result<(), SequencerNotReadyDetails> {
        // We cannot accept transactions until the latest finalized slot number
        // is AT LEAST 1. Meaning, as long as we're stuck at genesis, we can't
        // accept any transactions.
        if self.latest_info.latest_finalized_slot_number == SlotNumber::GENESIS {
            return Err(SequencerNotReadyDetails::WaitingOnDa {
                finalized_slot_number: SlotNumber::GENESIS,
                needed_finalized_slot_number: SlotNumber::new(1),
            });
        }

        let batch_status = self.batch_blob_sender_status();
        if batch_status.is_busy() {
            return Err(SequencerNotReadyDetails::WaitingOnBlobSender {
                max_concurrent_batch_blobs,
                nb_of_batch_blobs_in_flight: batch_status.in_flight,
            });
        }

        if let Some(height_to_stop_at) = height_to_stop_at {
            let current_height = self.current_height();
            if current_height >= height_to_stop_at {
                return Err(SequencerNotReadyDetails::PreferredSequencerAtStopHeight {
                    current_height,
                    height_to_stop_at,
                });
            }
        }

        self.start_replica_task_notifier
            .check_replica_status_or_ok_for_leader()?;

        self.is_ready.as_ref().map_err(|details| details.clone())?;
        Ok(())
    }

    pub(crate) fn is_replica_role(&self) -> bool {
        self.seq_role == SequencerRole::PgSyncReplica
    }

    /// Create a new batch, if possible. Errors here are expected, because it's not always possible to create a new batch due to transient DA issues.
    /// We can only create a new batch if we have a finalized slot available to use as our `visible_slot_number_after_increase`.
    #[tracing::instrument(skip_all, level = "trace")]
    pub(crate) async fn try_to_create_and_start_batch_if_none_in_progress(
        &mut self,
        leave_space_for_next_batch: bool,
    ) -> Result<(), BatchCreationError> {
        if self.executor.has_in_progress_batch() {
            return Ok(());
        }

        let visible_increase = match next_visible_slot_number_increase(
            &self.executor.checkpoint,
            &self.latest_info,
            leave_space_for_next_batch,
            self.seq_config
                .sequencer_kind_config
                .ideal_lag_behind_finalized_slot,
        ) {
            Ok(visible_increase) => visible_increase,
            Err(e) => {
                warn!(details = ?e, "A batch was requested but the sequencer is not ready to produce one.");
                return Err(BatchCreationError::NoFinalizedSlotAvailable);
            }
        };

        debug!(visible_increase, "No in-progress batch, starting a new one");

        let visible_slot_number_after_increase = self
            .executor
            .checkpoint
            .current_visible_slot_number()
            .advance(visible_increase.get().into());

        let maybe_seq_nr = self
            .do_batch_start(visible_slot_number_after_increase, visible_increase)
            .await?;

        if let Some(sequence_number) = maybe_seq_nr {
            tracing::debug!(
                %visible_increase,
                %visible_slot_number_after_increase,
                %sequence_number,
                "Sequencer created a new batch"
            );
        }

        Ok(())
    }

    pub(crate) fn new_executor_with_empty_uncommitted_changes(
        &self,
        info: &StateUpdateInfo<S::Storage>,
    ) -> RollupBlockExecutor<S, Rt> {
        let transaction_cache_write_handle = self.tx_cache_writer.clone();
        RollupBlockExecutor::<_, Rt>::new_with_tx_cache_writer(
            info,
            transaction_cache_write_handle,
            self.rollup_exec_config.clone(),
            self.seq_config.clone(),
            Default::default(),
        )
    }

    fn batch_blob_sender_status(&self) -> BlobSenderStatus {
        // Only batch blobs gate batch production.
        BlobSenderStatus {
            in_flight: self.nb_of_concurrent_batch_blob_submissions(),
            max_concurrent: self.seq_config.max_concurrent_batch_blobs,
        }
    }

    pub(crate) fn proof_blob_sender_status(&self) -> BlobSenderStatus {
        BlobSenderStatus {
            in_flight: self.in_flight_proof_blobs.load(Ordering::Relaxed),
            max_concurrent: self.max_concurrent_proof_blobs,
        }
    }

    fn node_root_hash(&self) -> anyhow::Result<<S::Storage as Storage>::Root> {
        self.latest_info
            .storage
            .get_root_hash(self.latest_info.slot_number)
            .with_context(|| {
                format!(
                    "missing root hash for committed slot {}",
                    self.latest_info.slot_number
                )
            })
    }

    fn current_height(&self) -> RollupHeight {
        self.executor.checkpoint.rollup_height_to_access()
    }
}

// Methods in this block are shared between Master and Replica.
impl<S, Rt> Inner<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub(crate) async fn do_batch_start(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_increase: NonZero<u8>,
    ) -> Result<Option<SequenceNumber>, BatchCreationError> {
        if self.executor.has_in_progress_batch() {
            return Ok(None);
        }

        if let Some(height_to_stop_at) = self.stop_at_rollup_height {
            let current_height = self.current_height();
            if current_height >= height_to_stop_at {
                debug!(%current_height, %height_to_stop_at,"The sequencer is at stop height and tried to create a batch (aborted due to stop height).");
                return Err(BatchCreationError::PreferredSequencerAtStopHeight {
                    current_height,
                    height_to_stop_at,
                });
            }
        }

        if self.batch_blob_sender_status().is_busy() {
            warn!("The blob sender is busy, no batch could be started at this time.");
            return Err(BatchCreationError::BlobSenderBusy);
        }

        let node_state_root = self
            .node_root_hash()
            .map_err(BatchCreationError::DatabaseError)?;

        // DB operations handled by replica-aware db implementation
        let sequence_number = self.assign_sequence_number_to_batch();
        let min_profit_per_tx = self.seq_config.sequencer_kind_config.minimum_profit_per_tx;
        let proofs_to_replay = self
            .executor_events_sender
            .fetch_proofs_for_replay(sequence_number);

        let start_block_data = StartBlockData {
            sanity_check_visible_slot_number_after_increase: visible_slot_number_after_increase,
            visible_increase,
            node_state_root: node_state_root.clone(),
            minimum_profit_per_tx: min_profit_per_tx,
            is_responsible_for_gating_admins: !self.is_replica_role(),
            proofs_to_replay,
        };

        let old_checkpoint = self
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache();

        self.executor
            .start_rollup_block(start_block_data.clone())
            .await;

        let state_roots = self.executor.state_roots.clone();

        if state_roots.len() > 50 {
            tracing::warn!(state_roots = %state_roots.len(), "Executor: The computed state roots map is large, and cloning it can be costly in terms of time.");
        }

        let notification = StartBlockNotification {
            state_roots,
            data: start_block_data,
            checkpoint: old_checkpoint,
            sequence_number,
        };

        self.pi_controller.batch_start_time = std::time::Instant::now();

        self.cache_warm_up_executor
            .send_batch_start_notification(notification);

        self.executor_events_sender
            .start_batch(
                visible_slot_number_after_increase,
                visible_increase,
                sequence_number,
                self.executor
                    .checkpoint
                    .clone_with_empty_witness_dropping_temp_cache(),
            )
            .await;

        Ok(Some(sequence_number))
    }

    pub(crate) async fn do_new_tx(
        &mut self,
        tx_hash: TxHash,
        baked_tx: FullyBakedTx,
    ) -> (
        Result<
            (
                oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>,
                <S as Spec>::Gas,
            ),
            DoNewTxError<S>,
        >,
        ResourceUsed<S::Gas>,
    ) {
        // Even if this method return early it uses 1 request slot.
        let request_used = ResourceUsed::new(1, 0, 0, <S as Spec>::Gas::zero());

        if self.shutdown_receiver.has_changed().unwrap_or(true) {
            tracing::info!("The sequencer is shutting down. Cannot accept transactions");
            return (Err(DoNewTxError::Shutdown), request_used);
        }

        if !self.executor.has_in_progress_batch() {
            panic!(
                "No batch in progress, and no batch could be started. Please report this bug. {:?} {:?}",
                &self.executor.checkpoint, self.latest_info
            );
        }

        let sequence_number = self
            .sequence_number_of_open_batch
            .expect("No batch in progress in Inner::do_new_tx");
        let Inner {
            executor,
            batch_size_tracker,
            executor_events_sender,
            cache_warm_up_executor,
            ..
        } = &mut *self;

        let tx_len = baked_tx.len();
        if !batch_size_tracker.can_fit_tx_bytes(tx_len) {
            return (
                Err(DoNewTxError::TxTooBig {
                    current_batch_size: batch_size_tracker.current_batch_size,
                    max_batch_size: batch_size_tracker.max_batch_size,
                    tx_len,
                }),
                request_used,
            );
        }

        let baked_tx = cache_warm_up_executor.send_tx(baked_tx.clone(), sequence_number);
        let apply_tx_res = executor.apply_tx_to_in_progress_batch(baked_tx).await;

        let (
            AcceptedTxWithBudgetInfo {
                accepted_tx,
                remaining_slot_gas,
                execution_time_micros,
            },
            tx_changes,
        ) = match apply_tx_res {
            Ok(res) => {
                assert_eq!(
                    tx_hash, res.0.accepted_tx.tx_hash,
                    "The executor returned a different tx hash than expected"
                );
                res
            }
            Err(RollupBlockExecutorErrorWithBudget {
                inner_err: err,
                execution_time_micros,
                gas_used,
            }) => {
                tracing::debug!(%tx_hash, %err, "Transaction was dropped by the sequencer");

                let resource_used = ResourceUsed::new(1, tx_len, execution_time_micros, gas_used);
                return (Err(DoNewTxError::ExecutorError(err)), resource_used);
            }
        };

        let gas_used = accepted_tx.confirmation.gas_used();
        let finalized_tx_len = accepted_tx.tx.len();
        let resource_used = ResourceUsed::new(1, tx_len, execution_time_micros, gas_used);

        batch_size_tracker.add_tx(finalized_tx_len, execution_time_micros);
        let rx = executor_events_sender
            .send_accept_tx(accepted_tx, tx_changes, sequence_number)
            .await;

        (Ok((rx, remaining_slot_gas)), resource_used)
    }

    // Implements the I part of a PID controller for the batch size limit; we track our error rate over each batch and tune the bias
    // (does our controller over or under shoot the ideal rate?)
    #[allow(clippy::float_arithmetic)]
    fn update_size_limit_bias_on_batch_close(&mut self) {
        let (target_size, target_rate_bytes_per_sec) = self.get_target_batch_size_and_growth_rate();
        let target_size = target_size as f64;

        // How far we undershot our target batch size, expressed as a fraction of the target size.
        let error_fraction =
            (target_size - self.batch_size_tracker.current_batch_size as f64) / target_size;
        // The next bias is old bias + (error fraction * target rate) * the "learning rate" constant "K"
        let next_bias = self.pi_controller.size_limit_bias
            + BATCH_SIZE_LIMIT_BIAS_K_I * error_fraction * target_rate_bytes_per_sec;

        // Finally, we clamp the bias to be between -0.5 * target rate and 0.5 * target rate to prevent it from growing too large.
        let bias_min = -0.5 * target_rate_bytes_per_sec;
        let bias_max = 0.5 * target_rate_bytes_per_sec;
        self.pi_controller.size_limit_bias = next_bias.clamp(bias_min, bias_max);
    }

    // Implements the I part of a PID controller for the batch execution time limit. The I term is used to correct for systematic errors in our execution time limit.
    // It grows when we accept too few transactions over the batch lifetime, and shrinks when we accept too many.
    // Float arithmetic is allowed because of the offchain nature of rate limiting.
    #[allow(clippy::float_arithmetic)]
    fn update_execution_time_limit_bias_on_batch_close(&mut self) {
        // How many microseconds of execution time we'd like to spend on this batch.
        let (target_execution_time_micros, target_rate_micros_per_second) =
            self.get_target_execution_time_and_growth_rate();
        if target_execution_time_micros == 0 {
            return;
        }
        let target_execution_time_micros = target_execution_time_micros as f64;

        // How much time has elapsed since the batch was opened.
        let elapsed_secs = self
            .pi_controller
            .batch_start_time
            .elapsed()
            .min(self.pi_controller.approximate_block_time)
            .as_secs_f64();

        // How many microseconds of execution time we expect to have spent on this batch by now, if we were accepting txs at the target rate.
        let expected_execution_time_micros =
            (target_rate_micros_per_second * elapsed_secs).min(target_execution_time_micros);
        // How many microseconds of execution time we've actually spent on this batch.
        let actual_execution_time_micros =
            self.batch_size_tracker.batch_execution_time_micros as f64;
        // How far we are from our target execution time, expressed as a fraction of the target execution time.
        let error_fraction = (expected_execution_time_micros - actual_execution_time_micros)
            / target_execution_time_micros;

        // We use different learning rates for positive and negative errors.
        // Positive errors (we've been accepting too few transactions) should accumulate slowly because we don't want to
        // over-update and run out of space in the *next* batch. Running out of space is very bad - it causes complete downtime until the batch closes.
        // Negative errors (we've been accepting too many transactions) can react faster to an
        // over-full batch, because rejecting a bit too aggressively only causes the loss of low value txs.
        let k_i = if error_fraction.is_sign_negative() {
            EXECUTION_TIME_LIMIT_BIAS_NEGATIVE_ERROR_K_I
        } else {
            EXECUTION_TIME_LIMIT_BIAS_POSITIVE_ERROR_K_I
        };
        // The next bias is old bias + (error fraction * target rate) * the "learning rate" constant "K"
        let next_bias = self.pi_controller.execution_time_limit_bias
            + k_i * error_fraction * target_rate_micros_per_second;
        // We clamp the bias to be between -0.5 * target rate and 0.1 * target rate.
        // As before, we're fine with large negative biases (rejecting too aggressively), but we want to be careful about positive biases (accepting too many transactions).
        let bias_min = -0.5 * target_rate_micros_per_second;
        let bias_max = 0.1 * target_rate_micros_per_second;
        self.pi_controller.execution_time_limit_bias = next_bias.clamp(bias_min, bias_max);
    }

    // Returns the target batch size in bytes and the growth rate for the batch size (in bytes per second).
    // Returns a tuple (target, rate)
    #[allow(clippy::float_arithmetic)]
    pub(crate) fn get_target_batch_size_and_growth_rate(&self) -> (u64, f64) {
        let target_size = (self.batch_size_tracker.max_batch_size as u64)
            .checked_div(20)
            .and_then(|x| x.checked_mul(19))
            .unwrap_or(0);
        let target_rate =
            target_size as f64 / self.pi_controller.approximate_block_time.as_secs_f64();
        (target_size, target_rate)
    }

    // Returns the target execution time in microseconds and the growth rate for the execution time (in micros per second).
    // Returns a tuple (target, rate)
    #[allow(clippy::float_arithmetic)]
    pub(crate) fn get_target_execution_time_and_growth_rate(&self) -> (u64, f64) {
        let target_execution_time_micros = self
            .batch_execution_time_limit_micros
            .checked_div(20)
            .and_then(|x| x.checked_mul(19))
            .unwrap_or(0);
        let target_rate = target_execution_time_micros as f64
            / self.pi_controller.approximate_block_time.as_secs_f64();
        (target_execution_time_micros, target_rate)
    }

    #[allow(clippy::float_arithmetic)]
    fn reset_current_accept_rates_on_batch_close(&mut self) {
        // Update the bytes per second target rate with the latest bias from the PI controller.
        let (_, target_rate) = self.get_target_batch_size_and_growth_rate();
        self.pi_controller.current_tx_accept_rate_bytes_per_second =
            (target_rate + self.pi_controller.size_limit_bias).max(0.0);

        // Update the micros per second target rate with the latest bias from the PI controller.
        let (_, target_rate_micros_per_second) = self.get_target_execution_time_and_growth_rate();
        self.pi_controller
            .current_tx_accept_rate_execution_time_micros_per_second =
            (target_rate_micros_per_second + self.pi_controller.execution_time_limit_bias).max(0.0);
    }

    #[allow(clippy::float_arithmetic)]
    pub(crate) fn update_estimated_tx_execution_time_micros(&mut self, execution_time_micros: u64) {
        if execution_time_micros == 0 {
            return;
        }

        let execution_time_micros = execution_time_micros as f64;
        // If we haven't estimated the tx execution time yet, set it to the first measurement.
        if self.pi_controller.estimated_tx_execution_time_micros == 0.0 {
            self.pi_controller.estimated_tx_execution_time_micros = execution_time_micros;
            return;
        }
        // Otherwise, use an EWMA to estimate the tx execution time.
        self.pi_controller.estimated_tx_execution_time_micros =
            self.pi_controller.estimated_tx_execution_time_micros * 0.75
                + execution_time_micros * 0.25;
    }

    #[allow(clippy::float_arithmetic)]
    pub(crate) fn should_accept_load_shed_tx(&mut self, accept_probability: f64) -> bool {
        // If the accept probability is 1.0, we accept the tx but it has no impact on our rejection debt.
        // This means that we will reject txs slightly more aggressively when the probability drops again,
        // but it prevents bad behavior in case there are mixed probabilities of acceptance
        // (i.e. high prio txs get a guaranteed 1 while lower prio txs only have a 0.5 chance).
        if accept_probability >= 1.0 {
            return true;
        }
        // If the accept probability is 0.0 or less, we reject the tx
        if accept_probability <= 0.0 {
            return false;
        }

        // Otherwise, we add to our rejection debt based on the rejection probability. I.e. if we had a 20% chance of acceptance, we'd add 0.8 to our debt -
        // this tx on its own makes us almost due to reject another tx.
        self.pi_controller.load_shed_rejection_debt += 1.0 - accept_probability;

        // If the debt became greater than 1 (meaning we're due to reject a transaction) reject and decrease our debt
        // otherwise accept.
        if self.pi_controller.load_shed_rejection_debt >= 1.0 {
            self.pi_controller.load_shed_rejection_debt -= 1.0;
            false
        } else {
            true
        }
    }

    // Updates the PI controller based on the current batch size and execution time.
    fn update_pi_controller_on_batch_close(&mut self) {
        self.update_size_limit_bias_on_batch_close();
        self.update_execution_time_limit_bias_on_batch_close();
        self.reset_current_accept_rates_on_batch_close();
    }

    /// Closes the current batch.
    ///
    /// This should be called only when...
    /// 1. There's no more capacity to accept txs in the current batch.
    /// 2. We're absolutely sure we want to close the batch early even though we don't need to.
    ///
    /// Case 2 only happens when we've just finished updating the state *and* we have more than our ideal number of finalized slots available.
    #[tracing::instrument(skip_all, level = "trace")]
    pub(crate) async fn close_current_batch(&mut self) {
        // Terminate the batch.
        let forced_txs = self.executor.end_rollup_block().await;
        self.update_pi_controller_on_batch_close();
        self.batch_size_tracker = BatchSizeTracker::new(self.seq_config.max_batch_size_bytes);
        let checkpoint = self
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache();
        self.sequence_number_of_open_batch = None;
        self.executor_events_sender
            .close_batch(checkpoint, forced_txs)
            .await;
    }

    pub(crate) async fn process_proof(
        &mut self,
        blob_id: BlobInternalId,
        proof_bytes: PreferredProofDataBytes,
        sequence_number: SequenceNumber,
    ) {
        // Put the proof blob into the sequencer cache, from which it will get pulled out and processed when the next batch is created.
        // The side effects task also persists it to postgres.
        self.executor_events_sender
            .publish_proof_blob(blob_id, proof_bytes, sequence_number)
            .await;
    }
}
