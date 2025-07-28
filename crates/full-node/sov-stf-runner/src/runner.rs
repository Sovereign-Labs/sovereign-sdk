use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use jsonrpsee::RpcModule;
use sov_db::ledger_db::{LedgerDb, SlotCommit};
use sov_db::schema::{DeltaReader, SchemaBatch};
use sov_metrics::RunnerMetrics;
use sov_rollup_interface::common::{RollupHeight, SlotNumber};
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::ledger_api::LedgerStateProvider;
use sov_rollup_interface::node::{
    future_or_shutdown, DaSyncState, FutureOrShutdownOutput, SyncStatus,
};
use sov_rollup_interface::stf::{
    ExecutionContext, ProofOutcome, ProofReceipt, ProofReceiptContents, StateTransitionFunction,
    StoredEvent,
};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::{StateTransitionWitness, Zkvm};
use sov_rollup_interface::{ProvableHeightTracker, StateUpdateInfo};
use tokio::sync::watch;
use tracing::{debug, info, trace};

use crate::da_pre_fetcher::FinalizedBlocksBulkFetcher;
use crate::processes::{new_stf_info_channel, Receiver};
use crate::state_manager::StateManager;
use crate::{CorsConfiguration, MonitoringConfig, ProofManagerConfig, RunnerConfig};

type GenesisParams<ST, InnerVm, OuterVm, Da> =
    <ST as StateTransitionFunction<InnerVm, OuterVm, Da>>::GenesisParams;

type NextDaHeightToProcess = u64;

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
#[allow(clippy::type_complexity)]
pub struct StateTransitionRunner<Stf, Sm, Da, InnerVm, OuterVm>
where
    Da: DaService,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    Sm: HierarchicalStorageManager<Da::Spec>,
    Stf: StateTransitionFunction<InnerVm, OuterVm, Da::Spec>,
{
    first_unprocessed_height_at_startup: u64,
    da_polling_interval: Duration,
    da_service: Arc<Da>,
    da_height_at_genesis: u64,
    stf: Stf,
    state_manager: StateManager<Stf::StateRoot, Stf::Witness, Sm, Da>,
    listen_address_http: SocketAddr,
    stf_info_receiver: Option<Receiver<Stf::StateRoot, Stf::Witness, Da::Spec>>,
    sync_state: Arc<DaSyncState>,
    sync_fetcher: FinalizedBlocksBulkFetcher<Da>,
    shutdown_receiver: watch::Receiver<()>,
    secondary_shutdown_sender: watch::Sender<()>,
    background_handles: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
    start_at_rollup_height: Option<RollupHeight>,
    stop_at_rollup_height: Option<RollupHeight>,
    save_tx_bodies: bool,
}

struct DiscardEvents;
impl TryFrom<(u64, &StoredEvent)> for DiscardEvents {
    type Error = anyhow::Error;

    fn try_from(_value: (u64, &StoredEvent)) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

/// Initializes rollup genesis.
/// Gets proper DA block and finalizes storage.
/// Returns root hashes.
pub async fn initialize_state<Stf, InnerVm, OuterVm, Da, Sm>(
    stf: &Stf,
    storage_manager: &mut Sm,
    genesis_block: Da::FilteredBlock,
    genesis_params: GenesisParams<Stf, InnerVm, OuterVm, Da::Spec>,
) -> anyhow::Result<Stf::StateRoot>
where
    Stf: StateTransitionFunction<InnerVm, OuterVm, Da::Spec>,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    Da: DaService,
    Sm: HierarchicalStorageManager<
        Da::Spec,
        LedgerChangeSet = SchemaBatch,
        LedgerState = DeltaReader,
        StfState = Stf::PreState,
        StfChangeSet = Stf::ChangeSet,
    >,
{
    let block_header = genesis_block.header().clone();
    info!(
        header = %block_header.display(),
        "No history detected. Initializing chain on the block header..."
    );
    let (stf_state, ledger_state) = storage_manager.create_state_for(&block_header)?;
    let ledger_db = LedgerDb::with_reader(ledger_state)?;

    let (genesis_state_root, initialized_storage) =
        stf.init_chain(&block_header, stf_state, genesis_params);

    let data_to_commit: SlotCommit<_, Stf::BatchReceiptContents, Stf::TxReceiptContents> =
        SlotCommit::new(genesis_block, Vec::default());
    let mut ledger_change_set =
        ledger_db.materialize_slot(data_to_commit, genesis_state_root.as_ref())?;

    let finalized_slot_changes =
        ledger_db.materialize_latest_finalize_slot(SlotNumber::GENESIS, SlotNumber::GENESIS)?;

    ledger_change_set.merge(finalized_slot_changes);
    storage_manager.save_change_set(&block_header, initialized_storage, ledger_change_set)?;
    storage_manager.finalize(&block_header)?;

    Ok(genesis_state_root)
}

impl<Stf, Sm, Da, InnerVm, OuterVm> StateTransitionRunner<Stf, Sm, Da, InnerVm, OuterVm>
where
    Da: DaService<Error = anyhow::Error>,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    Sm: HierarchicalStorageManager<
        Da::Spec,
        LedgerChangeSet = SchemaBatch,
        LedgerState = DeltaReader,
    >,
    Sm::StfState: Clone,
    Stf: StateTransitionFunction<
        InnerVm,
        OuterVm,
        Da::Spec,
        PreState = Sm::StfState,
        ChangeSet = Sm::StfChangeSet,
    >,
{
    /// Creates a new [`StateTransitionRunner`].
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub async fn new(
        runner_config: RunnerConfig,
        pm_config: Option<ProofManagerConfig<Stf::Address>>,
        da_service: Arc<Da>,
        ledger_db: LedgerDb,
        stf: Stf,
        storage_manager: Sm,
        state_update_channel: watch::Sender<StateUpdateInfo<Sm::StfState>>,
        prev_state_root: Stf::StateRoot,
        sync_status_sender: watch::Sender<SyncStatus>,
        state_height_tracker: Box<dyn ProvableHeightTracker>,
        shutdown_receiver: watch::Receiver<()>,
        monitoring_config: MonitoringConfig,
        start_at_rollup_height: Option<RollupHeight>,
        stop_at_rollup_height: Option<RollupHeight>,
    ) -> anyhow::Result<Self> {
        error_if_tokio_runtime_is_not_multi_threaded()?;

        let axum_config = &runner_config.http_config;

        let listen_address_http =
            SocketAddr::new(axum_config.bind_host.parse()?, axum_config.bind_port);

        let next_item_numbers = ledger_db.get_next_items_numbers()?;
        let last_slot_processed_before_shutdown = next_item_numbers.slot_number.saturating_sub(1);

        let da_height_processed =
            runner_config.genesis_height + last_slot_processed_before_shutdown.get();

        let first_unprocessed_height_at_startup = da_height_processed + 1;
        debug!(
            %last_slot_processed_before_shutdown,
            %runner_config.genesis_height,
            %first_unprocessed_height_at_startup,
            proof_manager_config = ?pm_config,
            "Initializing StfRunner");

        let (stf_info_sender, stf_info_receiver) = if let Some(config) = pm_config {
            let channel = new_stf_info_channel(
                ledger_db.clone(),
                config.max_number_of_transitions_in_memory,
                config.max_number_of_transitions_in_db,
            )
            .await?;

            (Some(channel.0), Some(channel.1))
        } else {
            (None, None)
        };

        let target_da_height = Self::get_target_block(&da_service, &stop_at_rollup_height)
            .await?
            .height();

        let sync_state = Arc::new(DaSyncState {
            synced_da_height: da_height_processed.into(),
            target_da_height: AtomicU64::new(target_da_height),
            sync_status_sender,
        });

        let da_polling_interval = Duration::from_millis(runner_config.da_polling_interval_ms);

        let state_manager = StateManager::new(
            storage_manager,
            ledger_db,
            prev_state_root,
            state_update_channel,
            stf_info_sender,
            state_height_tracker,
            sync_state.clone(),
            da_polling_interval,
        )?;

        let (sync_fetcher, fetcher_background_handle) = FinalizedBlocksBulkFetcher::new(
            da_service.clone(),
            first_unprocessed_height_at_startup,
            runner_config.get_concurrent_sync_tasks(),
            shutdown_receiver.clone(),
        )
        .await?;

        // This sender is not used immediately,
        // But when REST and RPC handlers start, sender is used to get another subscription.
        let (secondary_shutdown_sender, mut secondary_shutdown_receiver) = watch::channel(());
        secondary_shutdown_receiver.mark_unchanged();

        tokio::spawn(async move {
            sov_metrics::init_metrics_tracker(&monitoring_config);
        });

        Ok(Self {
            first_unprocessed_height_at_startup,
            da_polling_interval,
            da_service: da_service.clone(),
            da_height_at_genesis: runner_config.genesis_height,
            stf,
            state_manager,
            listen_address_http,
            sync_state,
            stf_info_receiver,
            sync_fetcher,
            shutdown_receiver,
            secondary_shutdown_sender,
            background_handles: vec![fetcher_background_handle],
            start_at_rollup_height,
            stop_at_rollup_height,
            save_tx_bodies: runner_config.save_tx_bodies,
        })
    }

    async fn get_target_block(
        da_service: &Da,
        stop_at_rollup_height: &Option<RollupHeight>,
    ) -> anyhow::Result<<Da::Spec as DaSpec>::BlockHeader> {
        // If we've entered the upgrade procedure, the rollup processes only finalized blocks.
        if stop_at_rollup_height.is_some() {
            da_service.get_last_finalized_block_header().await
        } else {
            da_service.get_head_block_header().await
        }
    }

    /// Subscribes to this runner's [`StateUpdateInfo`] channel, if enabled.
    ///
    /// Only one [`Receiver`] is allowed, and subsequent calls to this method
    /// will return [`None`].
    pub fn take_stf_info_receiver(
        &mut self,
    ) -> Option<Receiver<Stf::StateRoot, Stf::Witness, Da::Spec>> {
        self.stf_info_receiver.take()
    }

    /// Returns the [`DaSyncState`] of the node.
    pub fn da_sync_state(&self) -> Arc<DaSyncState> {
        self.sync_state.clone()
    }

    /// Starts an HTTP server with the provided router.
    pub async fn start_http_server(
        &mut self,
        router: axum::Router<()>,
        methods: RpcModule<()>,
        cors_configuration: CorsConfiguration,
    ) -> anyhow::Result<SocketAddr> {
        let (http_task_handle, rest_address) = crate::http::start_http_server(
            &self.listen_address_http,
            router,
            methods,
            self.secondary_shutdown_sender.subscribe(),
            cors_configuration,
        )
        .await?;

        self.background_handles.push(http_task_handle);

        Ok(rest_address)
    }

    /// Spawn a [`tokio::task`] that updates the sync status every `polling_interval`.
    fn spawn_sync_status_updater(
        &self,
        polling_interval: Duration,
        shutdown_receiver: watch::Receiver<()>,
    ) -> tokio::task::JoinHandle<()> {
        let sync_state = self.sync_state.clone();
        let da_service = self.da_service.clone();
        let stop_at_rollup_height = self.stop_at_rollup_height;

        tokio::task::spawn(async move {
            let mut interval = tokio::time::interval(polling_interval);
            debug!(
                interval_ms = interval.period().as_millis(),
                "Interval for polling sync DA height"
            );
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await; // Tick the interval once because it starts at 0ms.

            loop {
                match future_or_shutdown(
                    Self::get_target_block(&da_service, &stop_at_rollup_height),
                    &shutdown_receiver,
                )
                .await
                {
                    FutureOrShutdownOutput::Shutdown => break,
                    FutureOrShutdownOutput::Output(Err(error)) => {
                        tracing::info!(
                            ?error,
                            "Received error getting head block header, continue to next tick..."
                        );
                        continue;
                    }
                    FutureOrShutdownOutput::Output(Ok(header)) => {
                        let target_da_height = header.height();
                        if let Err(error) = sync_state.update_target(target_da_height) {
                            tracing::warn!(
                                target_da_height,
                                ?error,
                                "Received error updating target height, stopping background task"
                            );
                            break;
                        };
                        if let SyncStatus::Syncing {
                            synced_da_height,
                            target_da_height,
                        } = sync_state.status()
                        {
                            match target_da_height.checked_sub(synced_da_height) {
                                None => {
                                    trace!(
                                        target_da_height,
                                        synced_da_height,
                                        "Reorg happened, switch is in progress"
                                    );
                                }
                                Some(distance) => {
                                    if distance > 1 {
                                        info!(
                                            synced_da_height,
                                            target_da_height, "Sync in progress"
                                        );
                                    } else {
                                        trace!(
                                            synced_da_height,
                                            target_da_height,
                                            "Sync in progress"
                                        );
                                    }
                                }
                            };
                        }
                        future_or_shutdown(interval.tick(), &shutdown_receiver).await;
                    }
                }
            }
        })
    }

    /// Runs the rollup.
    pub async fn run_in_process(&mut self) -> anyhow::Result<()> {
        self.state_manager.startup().await?;

        let mut next_da_height = self.first_unprocessed_height_at_startup;

        let status_updater_handle = self
            .spawn_sync_status_updater(self.da_polling_interval, self.shutdown_receiver.clone());

        let start_at_rollup_height = self.start_at_rollup_height;
        let stop_at_rollup_height = self.stop_at_rollup_height;
        let shutdown_receiver = self.shutdown_receiver.clone();
        loop {
            if self.stop_at_rollup_height.is_some() {
                // Rollup is performing an upgrade procedure. We wait until the next_da_height is finalized.
                let is_shutting_down = Self::wait_until_next_da_height_finalized_or_shutdown(
                    self,
                    next_da_height,
                    &shutdown_receiver,
                )
                .await?;

                if is_shutting_down {
                    break;
                }
            }
            match future_or_shutdown(
                self.process_next_slot(
                    next_da_height,
                    &start_at_rollup_height,
                    &stop_at_rollup_height,
                ),
                &shutdown_receiver,
            )
            .await
            {
                FutureOrShutdownOutput::Shutdown => break,
                FutureOrShutdownOutput::Output(slot_result) => match slot_result? {
                    Some(next) => next_da_height = next,
                    None => {
                        break;
                    }
                },
            }
        }

        info!("Runner main loop is completed, keep shutting down...");
        if let Err(e) = self.secondary_shutdown_sender.send(()) {
            tracing::warn!(
                ?e,
                "Failed to send secondary shutdown signal. Happens if no HTTP handlers are running"
            );
        }
        info!("Secondary shutdown sent, waiting for status updater to stop...");
        status_updater_handle
            .await
            .context("Status update handler")?;

        let background_handles = std::mem::take(&mut self.background_handles);
        for handle in background_handles {
            let _ = handle.await?;
        }

        Ok(())
    }

    async fn wait_until_next_da_height_finalized_or_shutdown(
        &self,
        next_da_height: u64,
        shutdown_receiver: &watch::Receiver<()>,
    ) -> anyhow::Result<bool> {
        loop {
            match future_or_shutdown(
                self.da_service.get_last_finalized_block_number(),
                shutdown_receiver,
            )
            .await
            {
                FutureOrShutdownOutput::Shutdown => return Ok(true),
                FutureOrShutdownOutput::Output(finalized_block_height) => {
                    let finalized = finalized_block_height?;
                    if next_da_height > finalized {
                        info!("Waiting until {next_da_height} is finalized, current finalized is {finalized}");
                        tokio::time::sleep(self.da_polling_interval).await;
                        continue;
                    } else {
                        break;
                    }
                }
            }
        }

        Ok(false)
    }

    #[tracing::instrument(skip(self))]
    async fn process_next_slot(
        &mut self,
        mut next_da_height: NextDaHeightToProcess,
        start_at_rollup_height: &Option<RollupHeight>,
        stop_at_rollup_height: &Option<RollupHeight>,
    ) -> anyhow::Result<Option<NextDaHeightToProcess>> {
        let loop_start = std::time::Instant::now();
        let prev_state_root = self.get_state_root().clone();
        debug!("Requesting DA block");

        let mut transaction_count = 0;
        let mut batch_count = 0;
        let get_block_start = std::time::Instant::now();
        let filtered_block = if next_da_height <= self.sync_fetcher.last_finalized_height {
            // no reorg will happen for this height, it is safe to just pull it from the fetcher,
            // which could have this block fetcher already
            self.sync_fetcher.get_block_at(next_da_height).await?
        } else {
            // Requests height might re-org
            crate::da_utils::fetch_block_reorg_aware(
                self.da_service.as_ref(),
                self.sync_state.as_ref(),
                next_da_height,
                self.da_polling_interval,
            )
            .await?
        };
        let get_block_time = get_block_start.elapsed();

        let (stf_pre_state, filtered_block) = self
            .state_manager
            .prepare_storage(filtered_block, &self.da_service)
            .await
            .map_err(|e| {
                tracing::warn!(?e, "Error during prepare_storage");
                e
            })?;

        let filtered_block_header = filtered_block.header().clone();
        if next_da_height != filtered_block_header.height() {
            debug!(
                existing_next_da_height = next_da_height,
                new_next_da_height = filtered_block_header.height(),
                "Updating next_da_height after storage_manager, as reorg happened."
            );
            next_da_height = filtered_block_header.height();
            tracing::Span::current().record("new_next_da_height", next_da_height);
            self.sync_state
                .update_synced(next_da_height.saturating_sub(1));
        }

        // STF execution
        let stf_execution_start = std::time::Instant::now();
        let mut relevant_blobs = self.da_service.extract_relevant_blobs(&filtered_block);
        let batch_blobs = &mut relevant_blobs.batch_blobs;
        let proof_blobs = &relevant_blobs.proof_blobs;
        debug!(
            batch_blobs_count = batch_blobs.len(),
            next_da_height,
            current_state_root = hex::encode(prev_state_root.as_ref()),
            batch_blobs = ?batch_blobs
                .iter()
                .map(|b| format!(
                    "sequencer={} blob_hash=0x{}",
                    b.sender(),
                    hex::encode(b.hash())
                ))
                .collect::<Vec<_>>(),
            proof_blobs = ?proof_blobs
                .iter()
                .map(|b| format!(
                    "sequencer={} blob_hash=0x{}, len={}",
                    b.sender(),
                    hex::encode(b.hash()),
                    b.total_len()
                ))
                .collect::<Vec<_>>(),
            "Extracted relevant blobs"
        );
        let da_extraction_time = stf_execution_start.elapsed();

        let apply_slot_start = std::time::Instant::now();
        let slot_result = self.stf.apply_slot(
            self.state_manager.get_state_root(),
            stf_pre_state,
            Default::default(),
            &filtered_block_header,
            relevant_blobs.as_iters(),
            ExecutionContext::Node,
        );

        let apply_slot_time = apply_slot_start.elapsed();

        // --- Before destructuring the receipt, extract some data for metrics ---
        let batch_bytes_processed: u64 = relevant_blobs
            .batch_blobs
            .iter()
            .map(|b| b.verified_data().len() as u64)
            .sum();
        let proof_bytes_processed: u64 = relevant_blobs
            .proof_blobs
            .iter()
            .map(|b| b.verified_data().len() as u64)
            .sum();
        let proof_blobs_processed = slot_result.proof_receipts.len();
        tracing::trace!(
            ?apply_slot_time,
            batch_receipts = slot_result.batch_receipts.len(),
            %batch_bytes_processed,
            proof_receipts = proof_blobs_processed,
            "Apply slot completed"
        );
        // --- End metric extraction ---

        let get_relevant_proofs_start = std::time::Instant::now();
        // Get merkle proofs for the relevant blobs
        let relevant_proofs = self
            .da_service
            .get_extraction_proof(&filtered_block, &relevant_blobs)
            .await;
        let get_relevant_proofs_time = get_relevant_proofs_start.elapsed();
        // Handling executed data
        let mut data_to_commit = SlotCommit::new(filtered_block, slot_result.discarded_blobs);
        for mut receipt in slot_result.batch_receipts {
            if !self.save_tx_bodies {
                for tx in receipt.tx_receipts.iter_mut() {
                    tx.body_to_save = None;
                }
            }
            batch_count += 1;
            transaction_count += receipt.tx_receipts.len();
            data_to_commit.add_batch(receipt);
        }

        let transition_data: StateTransitionWitness<Stf::StateRoot, Stf::Witness, Da::Spec> =
            StateTransitionWitness {
                initial_state_root: self.get_state_root().clone(),
                final_state_root: slot_result.state_root.clone(),
                da_block_header: filtered_block_header.clone(),
                relevant_proofs,
                relevant_blobs,
                witness: slot_result.witness,
            };

        let aggregated_proofs =
            Self::collect_aggregated_proofs(slot_result.proof_receipts.into_iter());

        let processing_changes_start = std::time::Instant::now();
        self.state_manager
            .process_stf_changes(
                &self.da_service,
                self.da_height_at_genesis,
                slot_result.change_set,
                transition_data,
                data_to_commit,
                aggregated_proofs,
            )
            .await?;
        trace!("Stf changes processing is completed");

        // Updating counters and metrics
        self.sync_state.update_synced(next_da_height);
        debug!(
            time = ?loop_start.elapsed(),
            "Block execution complete"
        );
        // New influxdb metrics
        sov_metrics::track_metrics(|metrics| {
            let synced_da_height = self
                .sync_state
                .synced_da_height
                .load(std::sync::atomic::Ordering::Acquire);
            let target_da_height = self
                .sync_state
                .target_da_height
                .load(std::sync::atomic::Ordering::Acquire);
            let point = RunnerMetrics {
                sync_distance: target_da_height as i64 - synced_da_height as i64,
                da_height: next_da_height,
                get_block_time,
                batches_processed: batch_count,
                batch_bytes_processed,
                proofs_processed: proof_blobs_processed as u64,
                proof_bytes_processed,
                transactions_processed: transaction_count as u64,
                process_slot_time: loop_start.elapsed(),
                apply_slot_time,
                stf_transition_time: stf_execution_start.elapsed(),
                extract_blobs_time: da_extraction_time,
                extraction_proof_time: get_relevant_proofs_time,
                processing_changes_time: processing_changes_start.elapsed(),
            };
            metrics.track_runner_metrics(point);
        });

        // If the rollup is upgrading and the current height has reached the stop point,
        // halt further slot processing.
        if let Some(stop_at_rollup_height) = stop_at_rollup_height {
            if &slot_result.rollup_height == stop_at_rollup_height {
                info!("Stopping at rollup height: {}", stop_at_rollup_height);
                return Ok(None);
            }
            assert!(
                &slot_result.rollup_height < stop_at_rollup_height,
                "The rollup height ({}) must be less than the stop height ({})",
                slot_result.rollup_height,
                stop_at_rollup_height
            );
        }

        Ok(Some(next_da_height + 1))
    }

    /// Allows reading current state root
    pub fn get_state_root(&self) -> &Stf::StateRoot {
        self.state_manager.get_state_root()
    }

    /// Retrieve a handle for the underlying DA service
    pub fn da_service(&self) -> Arc<Da> {
        self.da_service.clone()
    }

    fn collect_aggregated_proofs(
        receipts: impl Iterator<
            Item = ProofReceipt<Stf::Address, Da::Spec, Stf::StateRoot, Stf::StorageProof>,
        >,
    ) -> Vec<SerializedAggregatedProof> {
        let mut aggregated_proofs: Vec<SerializedAggregatedProof> = Vec::new();
        for receipt in receipts {
            match receipt.outcome {
                ProofOutcome::Valid(ProofReceiptContents::AggregateProof(
                    _public_data,
                    raw_proof,
                )) => {
                    aggregated_proofs.push(raw_proof);
                }
                ProofOutcome::Valid(_) => {
                    tracing::info!("Not aggregated proof, probably running in a different mode. Will be fixed in the future.");
                }
                _ => {
                    tracing::error!(
                        outcome = ?receipt.outcome,
                        blob_hash = hex::encode(receipt.blob_hash),
                        "Invalid proof outcome");
                }
            }
        }

        aggregated_proofs
    }
}

/// Creates a new [`StateUpdateInfo`] with some storage state and chain data
/// queried from [`LedgerDb`].
pub async fn query_state_update_info<S>(
    ledger_db: &LedgerDb,
    storage: S,
) -> anyhow::Result<StateUpdateInfo<S>> {
    let slot_number = ledger_db.get_head_slot_number().await?;
    let next_event_number = ledger_db
        .get_latest_event_number()
        .await?
        .map(|x| x + 1)
        .unwrap_or_default();
    let next_tx_number = ledger_db.get_next_items_numbers()?.tx_number;
    let latest_finalized_slot_number = ledger_db
        .get_latest_finalized_slot_number()
        .await?
        .min(slot_number);

    Ok(StateUpdateInfo {
        storage,
        ledger_reader: ledger_db.clone_reader(),
        next_event_number,
        next_tx_number,
        slot_number,
        latest_finalized_slot_number,
    })
}

fn error_if_tokio_runtime_is_not_multi_threaded() -> anyhow::Result<()> {
    use tokio::runtime::{Handle, RuntimeFlavor};

    match Handle::current().runtime_flavor() {
            RuntimeFlavor::CurrentThread => Err(anyhow::anyhow!("A multi-threaded Tokio runtime is required to run the rollup node. Check your Tokio configuration. If you're testing node functionality, make sure your test uses `#[tokio::test(flavor = \"multi_thread\")]` or an equivalent configuration. Aborting.")),
            _ => Ok(())
        }
}
