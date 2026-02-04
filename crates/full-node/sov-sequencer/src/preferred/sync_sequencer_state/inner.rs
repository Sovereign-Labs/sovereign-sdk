use crate::metrics::{
    track_sequence_number, PreferredSequencerChannelMetrics, PreferredSequencerChannelMetricsBatch,
};
use crate::preferred::block_executor::{
    AcceptedTxWithBudgetInfo, RollupBlockExecutor, RollupBlockExecutorError,
};
use crate::preferred::block_executor::{RollupBlockExecutorErrorWithBudget, StartBlockData};
use crate::preferred::cache_warm_up_executor::{CacheWarmUpExecutor, StartBlockNotification};
use crate::preferred::comfortable_gas_limit;
use crate::preferred::db::{latest_finalized_sequence_number, SequencerRole};
use crate::preferred::executor_events::ExecutorEventsSender;
use crate::preferred::rate_limiter::ResourceUsed;
use crate::preferred::rate_limiter::SovRateLimiter;
use crate::preferred::sync_sequencer_state::EventReceiverStartNotifier;
use crate::preferred::AcceptedTx;
use crate::preferred::BatchSizeTracker;
use crate::preferred::RollupBlockExecutorConfig;
use crate::preferred::{
    current_visible_slot_number_according_to_node, get_next_sequence_number_according_to_node,
    is_lagging_less_than_ideal_amount, next_visible_slot_number_increase, BatchCreationError,
    Confirmation, LedgerDb, PreferredBatchToReplay, PreferredSequencerConfig,
    PreferredSequencerFetchBatchesToReplayMetrics, TxResultWriter,
};
use crate::{SequencerConfig, SequencerNotReadyDetails, SlotNumber, TxHash};
use sov_blob_storage::SequenceNumber;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::Gas;
use sov_modules_api::{
    FullyBakedTx, GasArray, GasSpec, Runtime, Spec, StateCheckpoint, StateUpdateInfo,
    VersionReader, VisibleSlotNumber,
};
use sov_state::pinned_cache::PinnedCache;
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
    // This ledgerdb is used specifically for REST API and websocket subscriptions.
    // The sequencer controls when it is updated to solve inconsistency issues,
    // See [`LedgerDb::with_shared_notifications`] for more details.
    pub(crate) api_ledger_db: LedgerDb,

    pub(crate) seq_config: SequencerConfig<S::Address, PreferredSequencerConfig<S::Address>>,
    pub(crate) shutdown_receiver: watch::Receiver<()>,
    pub(crate) shutdown_sender: watch::Sender<()>,

    pub(crate) executor: RollupBlockExecutor<S, Rt>,
    pub(crate) latest_info: StateUpdateInfo<S::Storage>,
    pub(crate) batch_execution_time_limit_micros: u64,
    pub(crate) batch_size_tracker: BatchSizeTracker,
    pub(crate) is_ready: Result<(), SequencerNotReadyDetails>,
    pub(crate) in_flight_blobs: Arc<AtomicUsize>,
    pub(crate) executor_events_sender: ExecutorEventsSender<S, Rt>,
    pub(crate) sequence_number_of_next_blob: SequenceNumber,
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
    pub(crate) fn nb_of_concurrent_blob_submissions(&self) -> usize {
        self.in_flight_blobs.load(Ordering::Acquire)
    }

    pub(crate) async fn overwrite_next_sequence_number_for_recovery(
        &mut self,
        sequence_number: SequenceNumber,
    ) {
        info!(%sequence_number, "Overwriting next sequence number");
        self.sequence_number_of_next_blob = sequence_number;
        track_sequence_number(self.sequence_number_of_next_blob);
    }

    pub(crate) fn current_sequence_number(&self) -> SequenceNumber {
        self.sequence_number_of_next_blob.checked_sub(1).expect("Sequence number underflow. Cannot get sequence number if no batch has ever been active. This is a bug, please report")
    }

    pub(crate) fn get_and_inc_next_sequence_number(&mut self) -> SequenceNumber {
        let sequence_number = self.sequence_number_of_next_blob;
        self.sequence_number_of_next_blob = self
            .sequence_number_of_next_blob
            .checked_add(1)
            .expect("Sequence number overflow; this should be unreachable for a few billion years");
        track_sequence_number(self.sequence_number_of_next_blob);
        sequence_number
    }

    pub(crate) async fn prune_sequencer_db(&mut self) {
        let next_sequence_number = self.sequence_number_of_next_blob;
        let latest_state_info = &self.latest_info;
        let mut runtime = Rt::default();
        let next_sequence_number_according_to_node =
            get_next_sequence_number_according_to_node(latest_state_info, &mut runtime);

        sov_metrics::track_metrics(|tracker| {
            tracker.submit_inline(
                "sov_rollup_sequence_number_delta",
                format!(
                    "delta={}i",
                    (next_sequence_number as i64) - (next_sequence_number_according_to_node as i64)
                ),
            );
        });

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

        // Replace executor state
        self.executor.replace_state(new_executor).await;

        // Replace API state
        let mut rt = Rt::default();
        let checkpoint = StateCheckpoint::new(info.storage.clone(), &rt.kernel(), None); // The api state doesn't need a copy of the pinned cache.
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

        // Since we're entering recovery, we don't re-use any of the uncommitted changes
        // Since we'll replace the executor when we exit recovery, we don't need to populate the pinned cache.
        let recovery_executor = self.new_executor_with_empty_uncommitted_changes(info, None);

        self.force_overwrite_state(info.clone(), recovery_executor)
            .await;

        info!(?info, current_visible_slot_number = %current_visible_slot_number_according_to_node::<S,Rt>(info), "Beginning sequencer recovery");
    }

    #[tracing::instrument(skip_all, level = "trace")]
    pub(crate) fn completed_batches_to_replay(
        &self,
        sequence_number: SequenceNumber,
        include_in_progress_batch: bool,
    ) -> (
        Vec<PreferredBatchToReplay>,
        PreferredSequencerFetchBatchesToReplayMetrics,
    )
    where
        S: Spec,
        Rt: Runtime<S>,
    {
        let start = std::time::Instant::now();
        let result = self
            .executor_events_sender
            .fetch_completed_blobs_by_sequence(sequence_number, include_in_progress_batch);
        let duration = start.elapsed();
        let metrics = PreferredSequencerFetchBatchesToReplayMetrics {
            duration,
            num_batches: result.len() as u64,
            num_transactions: result.iter().map(|b| b.batch.inner.data.len()).sum(),
        };
        (result, metrics)
    }

    /// Closes the current batch if it is nearly full (by gas limit) or has reached the target batch execution time.
    pub(crate) async fn close_batch_if_nearly_full(
        &mut self,
        remaining_slot_gas: <S as GasSpec>::Gas,
    ) {
        // Check if we're close to the gas limit and close the batch if we are.
        // We want to close when gas used is at least 95% of the initial gas limit.
        let initial_gas_limit = <S as GasSpec>::initial_gas_limit();
        let comfortable_gas_limit = comfortable_gas_limit::<S>();

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

        let in_flight_blobs = self.in_flight_blobs.load(Ordering::Relaxed);
        if in_flight_blobs >= COMFORTABLE_IN_FLIGHT_BLOBS {
            tracing::trace!(
                current_in_flight = %in_flight_blobs,
                max_comfortable = %COMFORTABLE_IN_FLIGHT_BLOBS,
                "Skipping batch production due too many in flight blobs");
            return;
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
            info!(
                "The sequencer is shutting down. Exiting trigger_batch_production_if_convenient."
            );
            return;
        }

        self.close_current_batch().await;
    }

    pub(crate) async fn check_readiness(
        &self,
        max_concurrent_blobs: usize,
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

        if let Some(nb_of_blobs_in_flight) = self.blob_sender_busy() {
            return Err(SequencerNotReadyDetails::WaitingOnBlobSender {
                max_concurrent_blobs,
                nb_of_blobs_in_flight,
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

    pub(crate) async fn update_api_ledger(&self, info: &StateUpdateInfo<S::Storage>) {
        let start = std::time::Instant::now();
        tracing::trace!(
            slot_number = %info.slot_number,
            latest_finalized_slot_number = %info.latest_finalized_slot_number,
            "Starting LedgerAPI storage update");
        self.api_ledger_db
            .replace_reader(info.ledger_reader.clone());
        tracing::trace!(
            time = ?start.elapsed(),
            slot_number = %info.slot_number,
            latest_finalized_slot_number = %info.latest_finalized_slot_number,
            "LedgerDb reader is replaced, sending notifications for the slot");
        self.api_ledger_db
            .send_notifications_for_slot(info.slot_number);
        tracing::trace!(
            time = ?start.elapsed(),
            slot_number = %info.slot_number,
            latest_finalized_slot_number = %info.latest_finalized_slot_number,
            "LedgerAPI storage updated, notification has been sent");

        self.tx_cache_writer.prune(info.next_tx_number).await;
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
        pinned_cache: Option<PinnedCache>,
    ) -> RollupBlockExecutor<S, Rt> {
        let transaction_cache_write_handle = self.tx_cache_writer.clone();
        RollupBlockExecutor::<_, Rt>::new_with_tx_cache_writer(
            info,
            transaction_cache_write_handle,
            self.rollup_exec_config.clone(),
            self.seq_config.clone(),
            Default::default(),
            pinned_cache,
        )
    }

    fn blob_sender_busy(&self) -> Option<usize> {
        let num_current_in_flight = self.nb_of_concurrent_blob_submissions();

        if num_current_in_flight > self.seq_config.max_concurrent_blobs {
            Some(num_current_in_flight)
        } else {
            None
        }
    }

    fn node_root_hash(&self) -> anyhow::Result<<S::Storage as Storage>::Root> {
        self.latest_info
            .storage
            .get_root_hash(self.latest_info.slot_number)
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

        if self.blob_sender_busy().is_some() {
            warn!("The blob sender is busy, no batch could be started at this time.");
            return Err(BatchCreationError::BlobSenderBusy);
        }

        let node_state_root = self
            .node_root_hash()
            .map_err(BatchCreationError::DatabaseError)?;

        // DB operations handled by replica-aware db implementation
        let sequence_number = self.get_and_inc_next_sequence_number();
        let min_profit_per_tx = self.seq_config.sequencer_kind_config.minimum_profit_per_tx;

        let start_block_data = StartBlockData {
            sanity_check_visible_slot_number_after_increase: visible_slot_number_after_increase,
            visible_increase,
            node_state_root: node_state_root.clone(),
            minimum_profit_per_tx: min_profit_per_tx,
            is_responsible_for_gating_admins: !self.is_replica_role(),
        };

        let old_checkpoint = self
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache_and_ignoring_pinned_cache();

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

        self.cache_warm_up_executor
            .send_batch_start_notification(notification);

        self.executor_events_sender
            .start_batch(
                visible_slot_number_after_increase,
                visible_increase,
                sequence_number,
                self.executor
                    .checkpoint
                    .clone_with_empty_witness_dropping_temp_cache_and_ignoring_pinned_cache(),
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

        let sequence_number = self.current_sequence_number();
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
        let resource_used = ResourceUsed::new(1, tx_len, execution_time_micros, gas_used);

        batch_size_tracker.add_tx(tx_len, execution_time_micros);
        let rx = executor_events_sender
            .send_accept_tx(accepted_tx, tx_changes, sequence_number)
            .await;

        (Ok((rx, remaining_slot_gas)), resource_used)
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
        self.batch_size_tracker = BatchSizeTracker::new(self.seq_config.max_batch_size_bytes);
        let checkpoint = self
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache_and_ignoring_pinned_cache();
        self.executor_events_sender
            .close_batch(checkpoint, forced_txs)
            .await;
    }
}
