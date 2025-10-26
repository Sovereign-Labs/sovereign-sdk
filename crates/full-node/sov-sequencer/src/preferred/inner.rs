#![allow(dead_code)]
use super::batch_size_tracker::BatchSizeTracker;
use crate::metrics::{
    track_sequence_number, PreferredSequencerChannelMetrics, PreferredSequencerChannelMetricsBatch,
    PreferredSequencerPruneMetrics, PreferredSequencerSlotNumberMetrics,
};
use crate::preferred::block_executor::StartBlockData;
use crate::preferred::block_executor::{
    AcceptedTxWithBudgetInfo, RollupBlockExecutor, RollupBlockExecutorError,
};
use crate::preferred::cache_warm_up_executor::{CacheWarmUpExecutor, StartBlockNotification};
use crate::preferred::db::{latest_finalized_sequence_number, BatchToStore};
use crate::preferred::executor_events::ExecutorEventsSender;
use crate::preferred::replica::db_data::DbData;
use crate::preferred::replica::event_handler::ReplicaError;
use crate::preferred::replica::replica_sync_task::DBDataRejected;
use crate::preferred::update_state::do_next_event;
use crate::preferred::RollupBlockExecutorConfig;
use crate::preferred::{
    current_visible_slot_number_according_to_node, exit_rollup,
    get_next_sequence_number_according_to_node, is_lagging_less_than_ideal_amount,
    next_visible_slot_number_increase, slot_count_delta_acceptable_lower_bound, AcceptedTx,
    BatchCreationError, Confirmation, DbEvent, LedgerDb, PreferredBatchToReplay,
    PreferredSeqOperation, PreferredSequencerConfig, PreferredSequencerFetchBatchesToReplayMetrics,
    PreferredSequencerReadBatch, TxResultWriter,
};
use crate::{SequencerConfig, SequencerNotReadyDetails, SlotNumber, TxHash};
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::macros::config_value;
use sov_modules_api::{
    FullyBakedTx, GasArray, GasSpec, Runtime, Spec, StateCheckpoint, StateUpdateInfo,
    VersionReader, VisibleSlotNumber,
};
use sov_state::{NativeStorage, Storage};
use std::collections::BTreeMap;
use std::num::NonZero;
use std::ops::Deref;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// These two constants are used to calculate the comfortable batch size limit.
/// Currently, this is 99% of the hard limit. After the comfortable limit is reached,
/// the sequencer will close and publish the current batch.
const COMFORTABLE_SIZE_LIMIT_MULTIPLIER: u64 = 99;
const COMFORTABLE_SIZE_LIMIT_DIVISOR: u64 = 100;

/// These two constants are used to calculate the comfortable gas limit.
/// Currently, this is 95% of the initial gas limit. After the comfortable limit is reached,
/// the sequencer will close and publish the current batch.
const COMFORTABLE_GAS_LIMIT_MULTIPLIER: u64 = 19;
const COMFORTABLE_GAS_LIMIT_DIVISOR: u64 = 20;
const COMFORTABLE_IN_FLIGHT_BLOBS: usize = 5;

const METRICS_BATCH_SIZE: usize = 32;

const CHANNEL_SIZE: usize = 128;

type AcceptTxRet<S, Rt> =
    Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>;

/// A inner sequencer struct containing state that requires synchronized access.
/// This struct accepts/rejects transactions, then hands them to the side effects task
/// to be persisted.
pub(crate) struct Inner<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    // This ledgerdb is used specifically for REST API and websocket subscriptions.
    // The sequencer controls when it is updated to solve inconsistency issues,
    // See [`LedgerDb::with_shared_notifications`] for more details.
    api_ledger_db: LedgerDb,

    seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
    shutdown_receiver: watch::Receiver<()>,
    shutdown_sender: watch::Sender<()>,

    executor: RollupBlockExecutor<S, Rt>,
    latest_info: StateUpdateInfo<S::Storage>,
    batch_execution_time_limit_micros: u64,
    batch_size_tracker: BatchSizeTracker,
    is_ready: Result<(), SequencerNotReadyDetails>,
    in_flight_blobs: Arc<AtomicUsize>,
    executor_events_sender: ExecutorEventsSender<S, Rt>,
    sequence_number_of_next_blob: SequenceNumber,
    /// A boolean that indicates whether the sequencer has finished its startup phase.
    /// We need this rather than relying on `SequencerNotReadyDetails::Startup` because that state
    /// can be overwritten when the node is resyncing.
    has_finished_startup: bool,
    metrics: Vec<PreferredSequencerChannelMetrics>,
    // Shared between sequencer and Inner.
    tx_queue_id: Arc<AtomicU64>,
    stop_at_rollup_height: Option<RollupHeight>,
    rollup_exec_config: RollupBlockExecutorConfig<S>,
    tx_cache_writer: TxResultWriter<S, Rt>,
    cache_warm_up_executor: CacheWarmUpExecutor<S>,
}

// We submit metrics when this guard is dropped.
struct InnerGuard<'a, S, Rt>
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
    fn nb_of_concurrent_blob_submissions(&self) -> usize {
        self.in_flight_blobs.load(Ordering::Acquire)
    }

    async fn overwrite_next_sequence_number_for_recovery(
        &mut self,
        sequence_number: SequenceNumber,
    ) {
        info!(%sequence_number, "Overwriting next sequence number");
        println!("OVERWRITE {:?}", sequence_number);
        self.sequence_number_of_next_blob = sequence_number;
        track_sequence_number(self.sequence_number_of_next_blob);
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

    fn current_sequence_number(&self) -> SequenceNumber {
        self.sequence_number_of_next_blob.checked_sub(1).expect("Sequence number underflow. Cannot get sequence number if no batch has ever been active. This is a bug, please report")
    }

    fn get_and_inc_next_sequence_number(&mut self) -> SequenceNumber {
        let sequence_number = self.sequence_number_of_next_blob;
        self.sequence_number_of_next_blob = self
            .sequence_number_of_next_blob
            .checked_add(1)
            .expect("Sequence number overflow; this should be unreachable for a few billion years");
        track_sequence_number(self.sequence_number_of_next_blob);
        sequence_number
    }

    async fn prune_sequencer_db(&mut self) {
        let next_sequence_number = self.sequence_number_of_next_blob;
        let latest_state_info = &self.latest_info;
        let mut runtime = Rt::default();
        let next_sequence_number_according_to_node =
            get_next_sequence_number_according_to_node(latest_state_info, &mut runtime);

        if self.is_replica() {
            println!(
                "XXXXX prune_sequencer_db next_sequence_number {:?} next_sequence_number_according_to_node {}",
                self.sequence_number_of_next_blob, next_sequence_number_according_to_node
            );
        }

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

    async fn force_overwrite_state(
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
        let checkpoint = StateCheckpoint::new(info.storage.clone(), &rt.kernel());
        self.executor_events_sender
            .force_update_api_state(checkpoint)
            .await;
    }

    async fn trigger_recovery(&mut self, info: &StateUpdateInfo<S::Storage>) {
        if self.is_replica() {
            // Replicas don't run recovery. We let the main sequencer run catchup. If we fail-over
            // midway, update_state() will automatically re-trigger recovery on this instance if
            // necessary - if the previous master already recovered enough then we'll just continue
            // operating.
            //
            // TODO: we do need to overwrite our state with the node's. Since recovery is expected
            // to be very rare, and if it does happen that means the rollup has already had
            // downtime and will already have had lost soft-confirmations, for now we'll require
            // the user to manually reset replicas.
            // To implement this properly we'd need to make sure we're 100% synced with the master
            // on exactly when to stop overwriting from the node and start applying new
            // transactions again. Probably by watching the `txs` table, so shouldn't be hard, but
            // not trivial enough to implement it on the spot.
            error!("We have encountered recovery conditions, but this is a replica sequencer. Recovery is currently unsupported for replicas. Please run a single master instance of the sequencer to restore the rollup to normal functionality. Wait for the rollup to be fully recovered, and then restart any replicas.");
            exit_rollup(&self.shutdown_sender).await;
            unreachable!();
        }

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
        let recovery_executor = self.new_executor_with_empty_uncommitted_changes(info);

        self.force_overwrite_state(info.clone(), recovery_executor)
            .await;

        info!(?info, current_visible_slot_number = %current_visible_slot_number_according_to_node::<S,Rt>(info), "Beginning sequencer recovery");
    }

    #[tracing::instrument(skip_all, level = "trace")]
    fn completed_batches_to_replay(
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
    async fn close_batch_if_nearly_full(&mut self, remaining_slot_gas: <S as GasSpec>::Gas) {
        // Check if we're close to the gas limit and close the batch if we are.
        // We want to close when gas used is at least 95% of the initial gas limit.
        let initial_gas_limit = <S as GasSpec>::initial_gas_limit();
        let gas_used = initial_gas_limit
            .checked_sub(remaining_slot_gas)
            .expect("remaining_lot_gas is always smaller than initial_gas_limit");
        let comfortable_gas_limit = initial_gas_limit
            .scalar_division(COMFORTABLE_GAS_LIMIT_DIVISOR)
            .checked_scalar_product(COMFORTABLE_GAS_LIMIT_MULTIPLIER).unwrap_or_else(|| {
                panic!(
                    "Cannot overflow after dividing by {COMFORTABLE_GAS_LIMIT_DIVISOR} and multiplying by {COMFORTABLE_GAS_LIMIT_MULTIPLIER}",
                )
            });
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
    async fn trigger_batch_production_if_convenient(&mut self) {
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

    async fn check_readiness(
        &self,
        max_concurrent_blobs: usize,
        height_to_stop_at: Option<RollupHeight>,
    ) -> Result<(), SequencerNotReadyDetails> {
        if self.is_replica() {
            return Err(SequencerNotReadyDetails::ReplicaMode);
        }

        // We cannot accept transactions until the latest finalized slot number
        // is AT LEAST 1. Meaning, as long as we're stuck at genesis, we can't
        // accept any transactions.
        if self.latest_info.latest_finalized_slot_number == SlotNumber::GENESIS {
            tracing::error!("Timed out while waiting for the node to progress beyond genesis. The sequencer can't accept transactions until that happens");
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

        self.is_ready.as_ref().map_err(|details| details.clone())?;
        Ok(())
    }

    fn is_replica(&self) -> bool {
        self.seq_config.sequencer_kind_config.is_replica
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
    async fn try_to_create_and_start_batch_if_none_in_progress(
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
                warn!(
                    "A batch was requested but the sequencer is not ready to produce one: {:?}",
                    e
                );
                return Err(BatchCreationError::NoFinalizedSlotAvailable);
            }
        };

        debug!(visible_increase, "No in-progress batch, starting a new one");

        let visible_slot_number_after_increase = self
            .executor
            .checkpoint
            .current_visible_slot_number()
            .advance(visible_increase.get().into());

        self.do_batch_start(visible_slot_number_after_increase, visible_increase)
            .await
    }

    fn new_executor_with_empty_uncommitted_changes(
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
}

// Methods in this block are shared between Master and Replica.
impl<S, Rt> Inner<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    async fn do_batch_start(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_increase: NonZero<u8>,
    ) -> Result<(), BatchCreationError> {
        if self.executor.has_in_progress_batch() {
            if self.is_replica() {
                panic!("RET BATCH IN PROGRESS");
            }

            return Ok(());
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

        if self.is_replica() {
            println!("BATCH START {:?}", sequence_number);
        }
        let min_profit_per_tx = self.seq_config.sequencer_kind_config.minimum_profit_per_tx;

        let start_block_data = StartBlockData {
            sanity_check_visible_slot_number_after_increase: visible_slot_number_after_increase,
            visible_increase,
            node_state_root: node_state_root.clone(),
            minimum_profit_per_tx: min_profit_per_tx,
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
            tracing::warn!("Executor: The computed state roots map is large, and cloning it can be costly in terms of time. state_roots len: {}", state_roots.len());
        }

        let notification = StartBlockNotification {
            state_roots,
            data: start_block_data,
            checkpoint: old_checkpoint,
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
                    .clone_with_empty_witness_dropping_temp_cache(),
            )
            .await;

        Ok(())
    }

    async fn do_new_tx(
        &mut self,
        tx_hash: TxHash,
        baked_tx: FullyBakedTx,
    ) -> Result<
        (
            oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>,
            <S as Spec>::Gas,
        ),
        DoNewTxError<S>,
    > {
        if self.shutdown_receiver.has_changed().unwrap_or(true) {
            tracing::info!("The sequencer is shutting down. Cannot accept transactions");
            return Err(DoNewTxError::Shutdown);
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

        let tx_len = baked_tx.data.len();
        if !batch_size_tracker.can_fit_tx_bytes(tx_len) {
            return Err(DoNewTxError::TxTooBig {
                current_batch_size: batch_size_tracker.current_batch_size,
                max_batch_size: batch_size_tracker.max_batch_size,
            });
        }

        let baked_tx = cache_warm_up_executor.send_tx(baked_tx.clone());
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
            Err(err) => {
                tracing::debug!(%tx_hash, %err, "Transaction was dropped by the sequencer");
                return Err(DoNewTxError::ExecutorError(err));
            }
        };

        batch_size_tracker.add_tx(tx_len, execution_time_micros);
        let rx = executor_events_sender
            .send_accept_tx(accepted_tx, tx_changes, sequence_number)
            .await;

        Ok((rx, remaining_slot_gas))
    }

    /// Closes the current batch.
    ///
    /// This should be called only when...
    /// 1. There's no more capacity to accept txs in the current batch.
    /// 2. We're absolutely sure we want to close the batch early even though we don't need to.
    ///
    /// Case 2 only happens when we've just finished updating the state *and* we have more than our ideal number of finalized slots available.
    #[tracing::instrument(skip_all, level = "trace")]
    async fn close_current_batch(&mut self) {
        // Terminate the batch.
        self.executor.end_rollup_block().await;
        self.batch_size_tracker = BatchSizeTracker::new(self.seq_config.max_batch_size_bytes);
        let checkpoint = self
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache();
        self.executor_events_sender.close_batch(checkpoint).await;
    }
}

#[derive(Debug)]
pub(crate) enum DoNewTxError<S: Spec> {
    TxTooBig {
        current_batch_size: usize,
        max_batch_size: usize,
    },
    ExecutorError(RollupBlockExecutorError<S>),
    Shutdown,
}

enum Message<S: Spec, Rt: Runtime<S>> {
    NextSequenceNumber {
        resp: oneshot::Sender<SequenceNumber>,
        reason: &'static str,
    },
    FetchCompletedBatches {
        resp: oneshot::Sender<FetchBatches>,
        next_sequence_number: u64,
        reason: &'static str,
    },
    SequencerConditions {
        resp: oneshot::Sender<PreferredSeqOperation<S, Rt>>,
        info: StateUpdateInfo<S::Storage>,
        next_sequence_number_according_to_node: u64,
        reason: &'static str,
    },
    CheckReadiness {
        resp: oneshot::Sender<Result<(), SequencerNotReadyDetails>>,
        max_concurrent_blobs: usize,
        height_to_stop_at: Option<RollupHeight>,
        reason: &'static str,
    },

    AcceptTx {
        resp: oneshot::Sender<AcceptTxRet<S, Rt>>,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        reason: &'static str,
    },
    LatestSlotNumber {
        resp: oneshot::Sender<SlotNumber>,
        reason: &'static str,
    },

    FinalCatchup {
        resp: oneshot::Sender<anyhow::Result<ProcessFinalCatchupData>>,
        info: StateUpdateInfo<S::Storage>,
        db_event_subscription: mpsc::Receiver<DbEvent>,
        executor: Box<RollupBlockExecutor<S, Rt>>,
        node_state_root: <S::Storage as Storage>::Root,
        data: ProcessFinalCatchupData,
        reason: &'static str,
    },
    PruneSequencerDb {
        reason: &'static str,
    },
    ForceOverwriteStateForRecovery {
        info: StateUpdateInfo<S::Storage>,
        reason: &'static str,
    },
    WaitNodeResync {
        info: StateUpdateInfo<S::Storage>,
        distance: u64,
        reason: &'static str,
    },
    #[cfg(feature = "test-utils")]
    ForceCloseCurrentBatch {
        reason: &'static str,
    },
    ProofBlob {
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        reason: &'static str,
    },
    TriggerBatchProductionIfConvenient {
        reason: &'static str,
    },

    SimpleStateUpdate {
        info: StateUpdateInfo<S::Storage>,
    },

    CloseCurrentBatch {
        reason: &'static str,
    },

    ReplicaBatchStartMsg {
        resp: oneshot::Sender<Result<(), ReplicaError<S>>>,
        batch_from_master: BatchToStore,
        reason: &'static str,
    },
    ReplicaNewTx {
        resp: oneshot::Sender<Result<(), ReplicaError<S>>>,
        seq_nr_from_master: u64,
        tx_hash: TxHash,
        baked_tx: FullyBakedTx,
        reason: &'static str,
    },

    ReplicaCloseCurrentBatch {
        resp: oneshot::Sender<Result<(), ReplicaError<S>>>,
        batch_from_master: BatchToStore,
        reason: &'static str,
    },
}

impl<S: Spec, Rt: Runtime<S>> Message<S, Rt> {
    fn is_accept_tx(&self) -> bool {
        matches!(self, Message::AcceptTx { .. })
    }

    fn priority(&self, runtime: &Rt) -> u64 {
        // Computes the priority of the message. Note that the implementation of the queue assumes that transactions never have higher priority than internal
        // sequencer messages - if we change this assumption, we'll need to update the implementation of sequencer state updater to handle this.
        match self {
            Message::AcceptTx { baked_tx, .. } => {
                let priority = runtime.get_transaction_priority(baked_tx);
                priority as u64
            }
            _ => u64::MAX,
        }
    }
}

#[derive(Debug)]
pub(crate) enum AcceptTxError<S: Spec> {
    SequencerOverloaded503,
    NotFullySynced(SequencerNotReadyDetails),
    BatchError {
        batch_creation_error: BatchCreationError,
        nb_of_concurrent_blob_submissions: usize,
    },
    NewTxError(DoNewTxError<S>),
}

#[derive(Debug)]
pub(crate) struct ProcessFinalCatchupData {
    pub(crate) batches_count: u64,
    pub(crate) transactions_count: usize,
    pub(crate) batch_is_in_progress: bool,
}

pub(crate) fn create<S, Rt>(
    api_ledger_db: LedgerDb,
    latest_info: StateUpdateInfo<S::Storage>,
    tx_queue_id: Arc<AtomicU64>,
    batch_execution_time_limit_micros: u64,
    seq_config: SequencerConfig<S::Address, PreferredSequencerConfig>,
    shutdown_receiver: watch::Receiver<()>,
    shutdown_sender: watch::Sender<()>,
    executor_events_sender: ExecutorEventsSender<S, Rt>,
    sequence_number_of_next_blob: SequenceNumber,
    in_flight_blobs: Arc<AtomicUsize>,
    stop_at_rollup_height: Option<RollupHeight>,
    rollup_exec_config: RollupBlockExecutorConfig<S>,
    tx_cache_writer: TxResultWriter<S, Rt>,
    cache_warm_up_executor: CacheWarmUpExecutor<S>,
) -> (
    SynchronizedSequencerState<S, Rt>,
    SequencerStateUpdator<S, Rt>,
)
where
    S: Spec,
    Rt: Runtime<S>,
{
    let (message_sender, message_receiver) = mpsc::channel(CHANNEL_SIZE);

    let is_ready = if seq_config.sequencer_kind_config.is_replica {
        Err(SequencerNotReadyDetails::ReplicaMode)
    } else {
        Err(SequencerNotReadyDetails::Startup)
    };

    let inner = Inner {
        api_ledger_db,
        executor: RollupBlockExecutor::new(
            &latest_info,
            rollup_exec_config.clone(),
            seq_config.clone(),
            Default::default(),
        ),
        latest_info,
        tx_queue_id,
        batch_execution_time_limit_micros,
        batch_size_tracker: BatchSizeTracker::new(seq_config.max_batch_size_bytes),
        seq_config: seq_config.clone(),
        shutdown_receiver: shutdown_receiver.clone(),
        shutdown_sender: shutdown_sender.clone(),
        executor_events_sender,
        sequence_number_of_next_blob,
        in_flight_blobs,
        has_finished_startup: false,
        metrics: Vec::with_capacity(128),
        is_ready,
        stop_at_rollup_height,
        rollup_exec_config,
        tx_cache_writer,
        cache_warm_up_executor,
    };

    let channel_size = Arc::new(AtomicU32::new(0));
    (
        SynchronizedSequencerState {
            inner,
            channel_size: channel_size.clone(),
            message_receiver,
            heap: BTreeMap::new(),
            runtime: Default::default(),
        },
        SequencerStateUpdator {
            message_sender,
            channel_size,
            shutdown_sender,
            shutdown_receiver,
        },
    )
}

pub(crate) struct SequencerStateUpdator<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    channel_size: Arc<AtomicU32>,
    message_sender: mpsc::Sender<Message<S, Rt>>,
    shutdown_sender: watch::Sender<()>,
    shutdown_receiver: watch::Receiver<()>,
}

#[derive(Debug)]
/// Describes errors that can occur when updating the sequencer state.
/// This type intentionally does *not* implement `std::error::Error` or `std::fmt::Debug` so that it cannot be directly converted to an `anyhow::Error`.
/// To convert to anyhow, first convert to a `StateUpdateError` or similar. This is done to ensure backward compatibility with existing code that
/// creates a local error type to represent shutdown and then attempts anyhow errors into that type to distinguish between shutdown and unexpected errors.
pub(crate) enum SequencerStateUpdatorError {
    Shutdown,
    Unexpected,
}

impl SequencerStateUpdatorError {
    /// Converts a [`SequencerStateUpdatorError`] into an [`anyhow::Error`] that can downcast into the correct "StateUpdateError::Shutdown" error type.
    /// This allows `UpdateState` to distinguish between graceful shutdowns and unexpected errors.
    pub fn into_state_update_error(self) -> anyhow::Error {
        match self {
            SequencerStateUpdatorError::Shutdown => crate::preferred::StateUpdateError::Shutdown.into(),
            SequencerStateUpdatorError::Unexpected => anyhow::anyhow!("The sequencer experienced an unexpected error and cannot accept transactions! See logs for more details."),
        }
    }
}

impl<S, Rt> SequencerStateUpdator<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub(crate) async fn next_sequence_number_msg(
        &self,
        reason: &'static str,
    ) -> Result<SequenceNumber, SequencerStateUpdatorError> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::NextSequenceNumber { resp, reason })
            .await?;

        self.recv(recv).await
    }

    pub(crate) async fn fetch_completed_batches_msg(
        &self,
        next_sequence_number: u64,
        reason: &'static str,
    ) -> Result<(FetchBatches, Duration), SequencerStateUpdatorError> {
        let start_time = std::time::Instant::now();
        let (resp, recv) = oneshot::channel();
        self.send(Message::FetchCompletedBatches {
            resp,
            next_sequence_number,
            reason,
        })
        .await?;

        Ok((self.recv(recv).await?, start_time.elapsed()))
    }

    pub(crate) async fn sequencer_conditions_msg(
        &self,
        info: &StateUpdateInfo<S::Storage>,
        next_sequence_number_according_to_node: u64,
        reason: &'static str,
    ) -> Result<PreferredSeqOperation<S, Rt>, SequencerStateUpdatorError> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::SequencerConditions {
            resp,
            info: info.clone(),
            next_sequence_number_according_to_node,
            reason,
        })
        .await?;

        self.recv(recv).await
    }

    pub(crate) async fn check_readiness_msg(
        &self,
        max_concurrent_blobs: usize,
        height_to_stop_at: Option<RollupHeight>,
        reason: &'static str,
    ) -> Result<Result<(), SequencerNotReadyDetails>, SequencerStateUpdatorError> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::CheckReadiness {
            resp,
            max_concurrent_blobs,
            height_to_stop_at,
            reason,
        })
        .await?;

        self.recv(recv).await
    }

    pub(crate) async fn accept_tx_msg(
        &self,
        baked_tx: &FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        reason: &'static str,
    ) -> Result<
        Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>,
        SequencerStateUpdatorError,
    > {
        let (resp, recv) = oneshot::channel();
        self.send(Message::AcceptTx {
            resp,
            baked_tx: baked_tx.clone(),
            tx_hash,
            original_tx_queue_id,
            reason,
        })
        .await?;

        self.recv(recv).await
    }

    pub(crate) async fn final_catchup_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
        db_event_subscription: mpsc::Receiver<DbEvent>,
        executor: Box<RollupBlockExecutor<S, Rt>>,
        node_state_root: <S::Storage as Storage>::Root,
        data: ProcessFinalCatchupData,
        reason: &'static str,
    ) -> Result<(anyhow::Result<ProcessFinalCatchupData>, Duration), SequencerStateUpdatorError>
    {
        let start_time = std::time::Instant::now();
        let (resp, recv) = oneshot::channel();
        self.send(Message::FinalCatchup {
            resp,
            info,
            db_event_subscription,
            executor,
            node_state_root,
            data,
            reason,
        })
        .await?;

        Ok((self.recv(recv).await?, start_time.elapsed()))
    }

    pub(crate) async fn prune_sequencer_db_msg(
        &self,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::PruneSequencerDb { reason }).await
    }

    pub(crate) async fn force_overite_state_for_recovery_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::ForceOverwriteStateForRecovery { info, reason })
            .await
    }

    pub(crate) async fn wait_for_node_resync_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
        distance: u64,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::WaitNodeResync {
            info,
            distance,
            reason,
        })
        .await
    }

    /// Closes the current batch
    #[cfg(feature = "test-utils")]
    pub(crate) async fn force_close_current_batch_msg(
        &self,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::ForceCloseCurrentBatch { reason }).await
    }

    pub(crate) async fn proof_blob_msg(
        &self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::ProofBlob {
            blob_id,
            data,
            reason,
        })
        .await
    }

    pub(crate) async fn trigger_batch_production_if_convenient_msg(
        &self,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::TriggerBatchProductionIfConvenient { reason })
            .await
    }

    pub(crate) async fn send_simple_state_update_msg(
        &self,
        info: StateUpdateInfo<S::Storage>,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::SimpleStateUpdate { info }).await
    }

    async fn send(&self, message: Message<S, Rt>) -> Result<(), SequencerStateUpdatorError> {
        self.channel_size.fetch_add(1, Ordering::Relaxed);
        if self.message_sender.send(message).await.is_err() {
            if self.shutdown_receiver.has_changed().unwrap_or(true) {
                info!("SynchronizedSequencerState(send) task exited, this is ok since the sequencer is shutting down.");
                return Err(SequencerStateUpdatorError::Shutdown);
            }
            return Err(SequencerStateUpdatorError::Unexpected);
        }
        Ok(())
    }

    async fn recv<T>(&self, recv: oneshot::Receiver<T>) -> Result<T, SequencerStateUpdatorError> {
        if let Ok(ret) = recv.await {
            Ok(ret)
        } else {
            if self.shutdown_receiver.has_changed().unwrap_or(true) {
                info!("SynchronizedSequencerState(recv) task exited, this is ok since the sequencer is shutting down.");
                return Err(SequencerStateUpdatorError::Shutdown);
            }
            error!("SynchronizedSequencerState(recv) task has shut down unexpectedly.");
            Err(SequencerStateUpdatorError::Unexpected)
        }
    }
    pub(crate) async fn close_current_batch_msg(
        &self,
        reason: &'static str,
    ) -> Result<(), SequencerStateUpdatorError> {
        self.send(Message::CloseCurrentBatch { reason }).await
    }

    pub(crate) async fn latest_slot_number_msg(
        &self,
        reason: &'static str,
    ) -> Result<SlotNumber, SequencerStateUpdatorError> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::LatestSlotNumber { resp, reason })
            .await?;

        self.recv(recv).await
    }

    pub(crate) async fn do_batch_start_msg_replica(
        &self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::ReplicaBatchStartMsg {
            resp,
            batch_from_master,
            reason,
        })
        .await?;

        self.recv(recv).await??;
        Ok(())
    }

    pub(crate) async fn do_new_tx_msg_replica(
        &self,
        seq_nr_from_master: u64,
        tx_hash: TxHash,
        baked_tx: FullyBakedTx,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::ReplicaNewTx {
            seq_nr_from_master,
            resp,
            tx_hash,
            baked_tx,
            reason,
        })
        .await?;

        self.recv(recv).await??;
        Ok(())
    }

    pub(crate) async fn close_current_batch_msg_replica(
        &self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let (resp, recv) = oneshot::channel();
        self.send(Message::ReplicaCloseCurrentBatch {
            resp,
            batch_from_master,
            reason,
        })
        .await?;
        self.recv(recv).await??;
        Ok(())
    }
}

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
    inner: Inner<S, Rt>,
    channel_size: Arc<AtomicU32>,
    message_receiver: mpsc::Receiver<Message<S, Rt>>,
    // A heap of message, ordered from low to high priority.
    heap: BTreeMap<Priority, Message<S, Rt>>,
    runtime: Rt,
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

            Message::FetchCompletedBatches {
                resp,
                next_sequence_number,
                reason,
            } => {
                let ret = self
                    .process_fetch_completed_batches(next_sequence_number, reason)
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
                reason,
            } => {
                let ret = self
                    .process_accept_tx(baked_tx, tx_hash, original_tx_queue_id, reason)
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
            Message::LatestSlotNumber { resp, reason } => {
                let ret = self.process_latest_slot_number(reason).await;
                self.send_response(resp, ret, "latest_slot_number").await;
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
            Message::ForceCloseCurrentBatch { reason: _reason } => {
                self.process_force_close_current_batch(_reason).await;
            }
            Message::ProofBlob {
                blob_id,
                data,
                reason,
            } => self.process_proof_blob(blob_id, data, reason).await,
            Message::TriggerBatchProductionIfConvenient { reason } => {
                self.process_trigger_batch_production_if_convenient(reason)
                    .await;
            }
            Message::SimpleStateUpdate { info } => {
                self.process_new_storage(info).await;
            }
            Message::CloseCurrentBatch { reason } => {
                self.process_close_current_batch(reason).await;
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
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "debug")]
    async fn get_inner_with_timing(&mut self, reason: &'static str) -> InnerGuard<S, Rt> {
        let channel_size = self.channel_size.fetch_sub(1, Ordering::Relaxed);
        InnerGuard::new(&mut self.inner, reason, channel_size)
    }

    async fn process_next_sequence_number(&mut self, reason: &'static str) -> SequenceNumber {
        let inner = self.get_inner_with_timing(reason).await;
        inner.sequence_number_of_next_blob
    }

    async fn process_fetch_completed_batches(
        &mut self,
        next_sequence_number: u64,
        reason: &'static str,
    ) -> FetchBatches {
        let mut inner = self.get_inner_with_timing(reason).await;

        let (completed_batches, metrics) =
            inner.completed_batches_to_replay(next_sequence_number, false);

        // Once we've caught up to the in-progress batch, we're done.
        let (db_events_sender, subscription) =
            mpsc::channel(inner.seq_config.sequencer_kind_config.db_event_channel_size);
        if completed_batches.is_empty() {
            inner
                .executor_events_sender
                .subscribe_to_events(db_events_sender);

            let fetch_in_progress_batch_time_start = std::time::Instant::now();
            let in_progress_batch = inner.executor_events_sender.fetch_in_progress_batch();
            let fetch_in_progress_batch_time = fetch_in_progress_batch_time_start.elapsed();

            drop(inner);
            return FetchBatches {
                metrics,
                flow: Flow::Break {
                    in_progress_batch,
                    subscription,
                    fetch_in_progress_batch_time,
                },
            };
        }

        drop(inner);
        FetchBatches {
            metrics,
            flow: Flow::Continue { completed_batches },
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
        let next_sequence_number = inner.sequence_number_of_next_blob;
        let ((batches_to_replay, fetch_batches_to_replay_metrics), is_startup) = {
            (
                inner.completed_batches_to_replay(next_sequence_number_according_to_node, true),
                !inner.has_finished_startup,
            )
        };

        let seq_visible_slot_number = match batches_to_replay.iter().last() {
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

        let condition_nodes_sequence_number_is_fresher =
            next_sequence_number_according_to_node > next_sequence_number;

        // There's an edge case on restart where the node hasn't synced to the chain tip yet but doesn't know it. We can check for it by seeing
        // if the first batch to replay has a sequencer number that's more than 1 greater than the node's sequence number (because sequence numbers are contiguous, and
        // we only prune a blob from the sequencer DB after it's been finalized on DA - so that blob must already exist on DA and the node just doesn't know that it isn't synced).
        let oldest_unfinalized_sequence_number = batches_to_replay
            .first()
            .map(|b: &PreferredBatchToReplay| b.batch.inner.sequence_number);
        let condition_node_is_unsynced_and_doesnt_know_it = (next_sequence_number_according_to_node
            .saturating_add(1))
            < oldest_unfinalized_sequence_number.unwrap_or(0);

        // Once we're this close to `deferred_slots_count`, we risk crossing the
        // `deferred_slots_count` threshold before the next call to
        // `update_state`. That's no good.
        let current_visible_slot_number =
            current_visible_slot_number_according_to_node::<S, Rt>(info);
        let condition_too_close_to_deferred_slots_count_for_comfort =
            info.slot_number.delta(current_visible_slot_number)
                > slot_count_delta_acceptable_lower_bound(
                    inner.seq_config.max_allowed_node_distance_behind,
                );

        // Resuming operations while the node is
        // lagging can cause issues e.g. during failover or after sequencer DB
        // deletion due to in-flight blobs that are not yet processed.
        let condition_node_is_lagging =
            distance > inner.seq_config.max_allowed_node_distance_behind;

        // Are there ANY soft confirmations to replay at all?
        // Note that we're holding a lock on the sequencer, so this is guaranteed to be up to date.
        let condition_are_there_batches_to_replay = !batches_to_replay.is_empty();

        let operation = match (
            condition_nodes_sequence_number_is_fresher,
            condition_too_close_to_deferred_slots_count_for_comfort,
            condition_node_is_lagging,
            condition_are_there_batches_to_replay,
            condition_node_is_unsynced_and_doesnt_know_it,
        ) {
            (true, _, _, true, _) => {
                if inner.is_replica() {
                    // TODO X1
                    inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                        target_da_height: sync_status.target_da_height(),
                        synced_da_height: sync_status.synced_da_height(),
                    });

                    // TODO X2
                    inner
                        .executor_events_sender
                        .flush_transactions_cache(info.next_tx_number)
                        .await;

                    println!(
                        "K0 WaitForNodeResyncToTip {:?} {:?}",
                        next_sequence_number_according_to_node, info
                    );

                    // TODO X3
                    inner.executor_events_sender.clean_all_batches_from_cache();
                    PreferredSeqOperation::WaitForNodeResyncToTip
                } else {
                    PreferredSeqOperation::Unreachable
                }
            }

            (true, _, false, false, _) => {
                warn!("The node has a higher sequence number than the sequencer, but we're very close to the chain tip, i.e. we don't expect to be simply syncing. This could mean there is another preferred sequencer running (which is not supported and will likely lead to issues), or you very recently restarted the node and there's still some in-flight blobs. Resyncing to the chain tip.");
                inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                    target_da_height: sync_status.target_da_height(),
                    synced_da_height: sync_status.synced_da_height(),
                });

                // TODO X4
                if inner.is_replica() {
                    inner
                        .executor_events_sender
                        .flush_transactions_cache(info.next_tx_number)
                        .await;

                    println!("K1 WaitForNodeResyncToTip");
                }
                PreferredSeqOperation::WaitForNodeResyncToTip
            }
            (_, _, true, _, _) => {
                warn!(?distance, "The sequencer must pause because the node has lagged behind the DA blockchain. This might lead to a brief downtime for users.");
                inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                    target_da_height: sync_status.target_da_height(),
                    synced_da_height: sync_status.synced_da_height(),
                });
                if inner.is_replica() {
                    println!("K2 WaitForNodeResyncWithAllowedSlack");
                }
                PreferredSeqOperation::WaitForNodeResyncWithAllowedSlack
            }
            (false, true, false, _, _) => {
                error!(
                    slot_number_according_to_node=%info.slot_number,
                    %current_visible_slot_number,
                    deferred_slots = %config_value!("DEFERRED_SLOTS_COUNT"),
                    "Sequencer has detected that it is past, or very close to, having the visible_slot_number lag behind the deferred_slots_count threshold. Normal operation will be suspended until this can be remedied.");

                inner.trigger_recovery(info).await;

                if inner.is_replica() {
                    println!("K3 RecoverAndCatchUp");
                }
                PreferredSeqOperation::RecoverAndCatchUp
            }
            // Node is out of sync and doesn't know it. This is a rare edge case after a DB wipe.
            (_, _, _, _, true) => {
                // Check for this condition after all of the normal "out-of-sync" conditions have been checked, because it may be possible for other unsynced conditions to trip this check
                // and we'd rather report the real root cause if there's a different one.
                warn!("The node is unsynced and doesn't know it. This probably means that you wiped the node DB and are resyncing.");
                inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                    target_da_height: sync_status.target_da_height(),
                    synced_da_height: sync_status.synced_da_height(),
                });
                if inner.is_replica() {
                    println!("K4 WaitForNodeResyncToTip");
                }
                PreferredSeqOperation::WaitForNodeResyncToTip
            }
            (false, false, false, _, _) => {
                let should_flush_tx_cache = is_startup || is_resync || is_recover;

                /*
                inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
                    target_da_height: sync_status.target_da_height(),
                    synced_da_height: sync_status.synced_da_height(),
                });*/

                // We only need to replay the transactions in the edge cases where the event/tx cache needs repopulating.
                // In all other cases, we can just accept the new storage and move on.
                let executor = if should_flush_tx_cache {
                    debug!(
                        is_startup,
                        is_resync,
                        is_recover,
                        "Proceeding with `replay_soft_confirmations_on_top_of_node_state`"
                    );

                    if inner.is_replica() {
                        println!(
                            "K5 ReplaySoftConfirmationsOnTopOfNodeStateIfNecessary {} {:?}",
                            next_sequence_number_according_to_node, info
                        );
                    }

                    inner
                        .executor_events_sender
                        .flush_transactions_cache(info.next_tx_number)
                        .await;

                    // On `should_flush_tx_cache` we have to refill the cache the first time we `replay_soft_confirmations_on_top_of_node_state`
                    Some(Box::new(
                        // Since we're replaying from the node state, don't reuse any uncommitted changes
                        inner.new_executor_with_empty_uncommitted_changes(info),
                    ))
                } else {
                    if inner.is_replica() {
                        println!("K6 ReplaySoftConfirmationsOnTopOfNodeStateIfNecessary  {:?} {next_sequence_number_according_to_node}", info);
                    }
                    let rollup_height =
                        StateCheckpoint::new(info.storage.clone(), &Rt::default().kernel())
                            .rollup_height_to_access();
                    debug!(
                        is_startup,
                        is_resync,
                        is_recover,
                        %rollup_height,
                        ?info,
                        "Skipping `replay_soft_confirmations_on_top_of_node_state`. Fast tracking info"
                    );
                    None
                };

                PreferredSeqOperation::ReplaySoftConfirmationsOnTopOfNodeStateIfNecessary(
                    executor,
                    time_spent_fetching_batches,
                )
            }
        };
        operation
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

    async fn process_latest_slot_number(&mut self, reason: &'static str) -> SlotNumber {
        let inner = self.get_inner_with_timing(reason).await;
        inner.latest_info.slot_number
    }

    async fn process_new_storage(&mut self, info: StateUpdateInfo<S::Storage>) {
        let mut inner = self.get_inner_with_timing("update_state::fast_path").await;
        // Atomically swap in the new storage and prune the old one.
        // Note that we use `StateCheckpoint::new(info.storage.clone(), ...)` *without* passing any intermediate state. This
        // is because we want to see what the height of the checkpoint we just received is, not the height of the sequencer's intermediate state.
        let new_rollup_height = StateCheckpoint::new(info.storage.clone(), &Rt::default().kernel())
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
        // Update the `inner`'s state to reflect the new storage.
        // These steps should match `process_final_catchup` except for the need to drop the db_event_subscription.
        inner.is_ready = Ok(());
        inner.has_finished_startup = true;
        inner.latest_info = info;
        let checkpoint = inner
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache();
        inner
            .executor_events_sender
            .force_update_api_state(checkpoint)
            .await;

        inner
            .executor
            .state_roots
            .retain(|height, _| *height > new_rollup_height);

        let info = &inner.latest_info;
        inner.update_api_ledger(info).await;
    }

    async fn process_final_catchup(
        &mut self,
        info: StateUpdateInfo<S::Storage>,
        mut db_event_subscription: mpsc::Receiver<DbEvent>,
        mut executor: Box<RollupBlockExecutor<S, Rt>>,
        node_state_root: <S::Storage as Storage>::Root,
        mut data: ProcessFinalCatchupData,
        reason: &'static str,
    ) -> anyhow::Result<ProcessFinalCatchupData> {
        let mut inner = self.get_inner_with_timing(reason).await;
        // Some events might come in while we're waiting to grab the lock.
        // Replay them.
        while let Ok(event) = db_event_subscription.try_recv() {
            if inner.shutdown_receiver.has_changed().unwrap_or(true) {
                tracing::info!("The sequencer is shutting down. Exiting replay_batch");
                return Ok(data);
            }

            do_next_event(
                &mut executor,
                event,
                &mut data.batches_count,
                &mut data.transactions_count,
                &node_state_root,
                &mut data.batch_is_in_progress,
            )
            .await?;
        }

        // The executor is now caught up. Swap it in
        inner.executor.replace_state(*executor).await;
        // Update the `inner`'s state to reflect the new storage.
        // These steps should match `process_new_storage` except for the need to drop the db_event_subscription.
        inner.is_ready = Ok(());
        inner.has_finished_startup = true;
        inner.latest_info = info;
        let checkpoint = inner
            .executor
            .checkpoint
            .clone_with_empty_witness_dropping_temp_cache();
        inner
            .executor_events_sender
            .force_update_api_state(checkpoint)
            .await;
        drop(db_event_subscription);

        let info = &inner.latest_info;
        inner.update_api_ledger(info).await;

        drop(inner);

        Ok(data)
    }

    async fn process_prune_sequencer_db(&mut self, reason: &'static str) {
        let start_prune = std::time::Instant::now();
        let mut inner = self.get_inner_with_timing(reason).await;
        if !inner.is_replica() {
            inner.trigger_batch_production_if_convenient().await;
        }
        inner.prune_sequencer_db().await;
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

        // Since we're entering recovery, we don't re-use any of the uncommitted changes
        let recovery_executor = inner.new_executor_with_empty_uncommitted_changes(&info);

        inner
            .force_overwrite_state(info.clone(), recovery_executor)
            .await;
        inner.update_api_ledger(&info).await;
    }

    async fn process_wait_for_node_resync(
        &mut self,
        info: StateUpdateInfo<S::Storage>,
        distance: u64,
        reason: &'static str,
    ) {
        let mut inner = self.get_inner_with_timing(reason).await;
        let mut rt = Rt::default();
        inner.is_ready = Err(SequencerNotReadyDetails::Syncing {
            target_da_height: info.sync_status.target_da_height(),
            synced_da_height: info.sync_status.synced_da_height(),
        });

        let node_sequence_number = get_next_sequence_number_according_to_node(&info, &mut rt);
        let our_sequence_number = inner.sequence_number_of_next_blob;

        if inner.is_replica() {
            println!(
                "XXXXX {:?} dist {distance} our {our_sequence_number} node {node_sequence_number}",
                info.sync_status
            );
        }

        if node_sequence_number > our_sequence_number {
            inner
                .overwrite_next_sequence_number_for_recovery(node_sequence_number)
                .await;
        }

        inner.latest_info = info.clone();
        // We update the API state, so users can query node state as it syncs.
        let checkpoint = StateCheckpoint::new(info.storage.clone(), &rt.kernel());
        inner
            .executor_events_sender
            .update_state_for_recovery(checkpoint)
            .await;

        let recovery_executor = inner.new_executor_with_empty_uncommitted_changes(&info);

        inner
            .force_overwrite_state(info.clone(), recovery_executor)
            .await;

        inner.update_api_ledger(&info).await;

        if inner.is_replica() && info.sync_status.distance() <= distance {
            // TODO X7
            inner.has_finished_startup = true;
            inner.is_ready = Ok(());
        }
    }

    /// Closes the current batch
    #[cfg(feature = "test-utils")]
    async fn process_force_close_current_batch(&mut self, reason: &'static str) {
        let mut inner = self.get_inner_with_timing(reason).await;
        inner.close_current_batch().await;
    }

    async fn process_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        reason: &'static str,
    ) {
        let mut inner = self.get_inner_with_timing(reason).await;
        let sequence_number = inner.get_and_inc_next_sequence_number();
        inner
            .executor_events_sender
            .publish_proof_blob(blob_id, data, sequence_number)
            .await;
    }

    async fn process_trigger_batch_production_if_convenient(&mut self, reason: &'static str) {
        // We don't run force_overwrite_state() here.
        // This is mostly fine, mainly the API state will be out of date until we've
        // finished sending our batches.
        // Adding parallel state update handling is not worth the complexity right now.

        let mut inner = self.get_inner_with_timing(reason).await;
        inner.trigger_batch_production_if_convenient().await;
    }

    async fn process_accept_tx(
        &mut self,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        reason: &'static str,
    ) -> Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>> {
        let mut inner = self.get_inner_with_timing(reason).await;

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

        let (rx, remaining_slot_gas) = inner
            .do_new_tx(tx_hash, baked_tx)
            .await
            .map_err(AcceptTxError::NewTxError)?;

        inner.close_batch_if_nearly_full(remaining_slot_gas).await;

        Ok(rx)
    }
    async fn process_close_current_batch(&mut self, reason: &'static str) {
        let mut inner = self.get_inner_with_timing(reason).await;
        inner.close_current_batch().await;
    }

    async fn process_do_batch_start_replica(
        &mut self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let mut inner = self.get_inner_with_timing(reason).await;

        let seq_nr_of_next_blob_for_this_executor = inner.sequence_number_of_next_blob;
        let seq_nr_from_master = batch_from_master.sequence_number;

        println!("BATCH S");
        validate_db_data_from_replica(
            inner.has_finished_startup,
            &inner.is_ready,
            DbData::BatchStart(batch_from_master),
            seq_nr_of_next_blob_for_this_executor,
            seq_nr_from_master,
        )?;

        println!("BATCH E");

        inner
            .do_batch_start(
                batch_from_master.visible_slot_number_after_increase,
                batch_from_master.visible_slots_to_advance,
            )
            .await?;

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
        let seq_nr_of_current_blob_for_this_executor = inner.current_sequence_number();

        println!("TX  seq_nr_from_master {seq_nr_from_master}, seq_nr_of_current_blob_for_this_executor {seq_nr_of_current_blob_for_this_executor}");

        //tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        validate_db_data_from_replica(
            inner.has_finished_startup,
            &inner.is_ready,
            DbData::Transaction(seq_nr_from_master, baked_tx.clone(), tx_hash),
            seq_nr_of_current_blob_for_this_executor,
            seq_nr_from_master,
        )?;
        println!("TX  VALIDATED");

        let _ = inner
            .do_new_tx(tx_hash, baked_tx)
            .await
            .map_err(ReplicaError::NewTx)?
            .0
            .await;

        Ok(())
    }

    async fn process_close_current_batch_replica(
        &mut self,
        batch_from_master: BatchToStore,
        reason: &'static str,
    ) -> Result<(), ReplicaError<S>> {
        let mut inner = self.get_inner_with_timing(reason).await;
        let seq_nr_of_current_blob_for_this_executor = inner.current_sequence_number();
        let seq_nr_from_master = batch_from_master.sequence_number;

        println!("");
        println!("BATCH END seq_nr_from_master {seq_nr_from_master} seq_nr_of_current_blob_for_this_executor {seq_nr_of_current_blob_for_this_executor}");

        validate_db_data_from_replica(
            inner.has_finished_startup,
            &inner.is_ready,
            DbData::BatchEnd(batch_from_master),
            seq_nr_of_current_blob_for_this_executor,
            seq_nr_from_master,
        )?;
        println!("VALIDATED BATCH");
        inner.close_current_batch().await;

        Ok(())
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
            ret,
        ));
    }

    if let Err(err) = is_ready {
        return Err(ReplicaError::NotReady(err.clone(), ret));
    }

    if seq_nr_for_this_executor > seq_nr_from_master {
        return Err(ReplicaError::Rejected(DBDataRejected::ExecutorAhead(
            seq_nr_for_this_executor,
        )));
    }

    if seq_nr_for_this_executor < seq_nr_from_master {
        return Err(ReplicaError::Rejected(DBDataRejected::ExecutorBehind(ret)));
    }

    Ok(())
}

#[derive(Debug)]
pub(crate) enum Flow {
    Break {
        in_progress_batch: Option<PreferredSequencerReadBatch>,
        subscription: mpsc::Receiver<DbEvent>,
        fetch_in_progress_batch_time: Duration,
    },
    Continue {
        completed_batches: Vec<PreferredBatchToReplay>,
    },
}

#[derive(Debug)]
pub(crate) struct FetchBatches {
    pub(crate) metrics: PreferredSequencerFetchBatchesToReplayMetrics,
    pub(crate) flow: Flow,
}
