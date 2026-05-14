use std::num::NonZero;
use std::sync::Arc;

use backon::{BackoffBuilder, ExponentialBuilder};
use sov_rollup_full_node_interface::DaSyncState;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput, SyncStatus};
use sov_rollup_interface::stf::ProofSender;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use types::{BlockProofInfo, BlockProofStatus, UnAggregatedProofList};

use self::types::AggregateProofMetadata;
use super::StateTransitionInfo;
use crate::processes::{CursorHandle, ProverService, Receiver};

mod types;

const BACKOFF_POLICY_MIN_DELAY: u64 = 1;
const BACKOFF_POLICY_MAX_DELAY: u64 = 60;
const BACKOFF_POLICY_MAX_NUM_RETRIES: usize = 5;

/// Manages the lifecycle of the `AggregatedProof`.
#[allow(clippy::type_complexity)]
pub struct ZkProofManager<Ps: ProverService> {
    prover_service: Arc<Ps>,
    proofs_to_create: UnAggregatedProofList<Ps>,
    aggregated_proof_block_jump: NonZero<usize>,
    eager_proof_submission: bool,
    max_number_of_aggregated_proofs_in_memory: NonZero<usize>,
    proof_sender: Box<dyn ProofSender>,
    backoff_policy: ExponentialBuilder,
    stf_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
    da_sync_state: Arc<DaSyncState>,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
    shutdown_sender: tokio::sync::watch::Sender<()>,
    start_fresh_outer_proof_on_resync: bool,
}

impl<Ps: ProverService> ZkProofManager<Ps>
where
    Ps::DaService: DaService<Error = anyhow::Error>,
{
    /// Creates a new proof manager.
    #[allow(clippy::type_complexity)]
    pub fn new(
        prover_service: Ps,
        aggregated_proof_block_jump: NonZero<usize>,
        eager_proof_submission: bool,
        max_number_of_aggregated_proofs_in_memory: NonZero<usize>,
        proof_sender: Box<dyn ProofSender>,
        stf_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
        da_sync_state: Arc<DaSyncState>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        shutdown_sender: tokio::sync::watch::Sender<()>,
        start_fresh_outer_proof_on_resync: bool,
    ) -> Self {
        Self {
            prover_service: Arc::new(prover_service),
            proofs_to_create: UnAggregatedProofList::new(),
            aggregated_proof_block_jump,
            eager_proof_submission,
            max_number_of_aggregated_proofs_in_memory,
            proof_sender,
            backoff_policy: ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(BACKOFF_POLICY_MIN_DELAY))
                .with_max_delay(Duration::from_secs(BACKOFF_POLICY_MAX_DELAY))
                .with_max_times(BACKOFF_POLICY_MAX_NUM_RETRIES),
            stf_info_receiver,
            da_sync_state,
            shutdown_receiver,
            shutdown_sender,
            start_fresh_outer_proof_on_resync,
        }
    }

    /// Spawns the intake and aggregator background tasks.
    ///
    /// Returns a single supervisor handle that resolves once both tasks
    /// terminate.
    pub async fn post_aggregated_proof_to_da_in_background(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            tracing::info!("Spawning an aggregated proof posting background task");

            let (metadata_tx, metadata_rx) = mpsc::channel::<(AggregateProofMetadata<Ps>, u64)>(
                self.max_number_of_aggregated_proofs_in_memory.get(),
            );

            // Cursor handle is shared: the aggregator advances it after a
            // publish succeeds, and the intake advances it for slots that are
            // intentionally skipped (e.g. the resync window). See module-level
            // docs.
            let cursor = self.stf_info_receiver.cursor_handle();

            let aggregator = AggregatorTask {
                prover_service: self.prover_service.clone(),
                proof_sender: self.proof_sender,
                backoff_policy: self.backoff_policy,
                metadata_rx,
                cursor: cursor.clone(),
                shutdown_receiver: self.shutdown_receiver.clone(),
                shutdown_sender: self.shutdown_sender,
            };
            let aggregator_handle = tokio::spawn(async move {
                if let Err(e) = aggregator.run().await {
                    tracing::error!(error = ?e, "Aggregator task failed");
                }
            });

            let intake = IntakeTask {
                prover_service: self.prover_service,
                proofs_to_create: self.proofs_to_create,
                aggregated_proof_block_jump: self.aggregated_proof_block_jump,
                eager_proof_submission: self.eager_proof_submission,
                stf_info_receiver: self.stf_info_receiver,
                da_sync_state: self.da_sync_state,
                metadata_tx,
                cursor,
                shutdown_receiver: self.shutdown_receiver,
                start_fresh_outer_proof_on_resync: self.start_fresh_outer_proof_on_resync,
            };
            if let Err(e) = intake.run().await {
                tracing::error!(error = ?e, "Intake task failed");
            }

            let _ = aggregator_handle.await;
        })
    }
}

/// Tracks the resync skip-window state machine for the proof intake.
enum ResyncState {
    /// `start_fresh_outer_proof_on_resync` is disabled; always prove.
    Disabled,
    /// Flag enabled, DA still `Syncing`. Skip-advance every slot so the runner
    /// can keep pushing transitions and sync can complete. Cutoff not yet captured.
    AwaitingSync,
    /// Flag enabled, first `Synced` observed at this DA height. Skip slots below
    /// this height (they were drained but never proved); prove slots at or above.
    SkipBelow(u64),
}

/// Per-slot intake side of [`ZkProofManager`]. See the type-level docs for
/// how it cooperates with [`AggregatorTask`].
struct IntakeTask<Ps: ProverService> {
    prover_service: Arc<Ps>,
    proofs_to_create: UnAggregatedProofList<Ps>,
    aggregated_proof_block_jump: NonZero<usize>,
    eager_proof_submission: bool,
    stf_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
    da_sync_state: Arc<DaSyncState>,
    metadata_tx: mpsc::Sender<(AggregateProofMetadata<Ps>, u64)>,
    cursor: CursorHandle,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
    start_fresh_outer_proof_on_resync: bool,
}

impl<Ps: ProverService> IntakeTask<Ps>
where
    Ps::DaService: DaService<Error = anyhow::Error>,
{
    async fn run(mut self) -> anyhow::Result<()> {
        let mut resync_state = if self.start_fresh_outer_proof_on_resync {
            ResyncState::AwaitingSync
        } else {
            ResyncState::Disabled
        };

        loop {
            match future_or_shutdown(self.stf_info_receiver.read_next(), &self.shutdown_receiver)
                .await
            {
                FutureOrShutdownOutput::Shutdown => {
                    tracing::info!("Shutting down aggregated proof intake task...");
                    break;
                }
                FutureOrShutdownOutput::Output(stf_info_result) => {
                    let stf_info = match stf_info_result? {
                        None => {
                            tracing::debug!("Received None instead of StateTransitionInfo. This can happen if the transition has already been processed by the `Receiver`. In that case, it is fine to ignore the notification.");
                            continue;
                        }
                        Some(stf_info) => stf_info,
                    };
                    tracing::trace!(
                        slot_number = %stf_info.slot_number(),
                        block_header = %stf_info.da_block_header().display(),
                        "Received STF info"
                    );

                    // If still observing resync, try to capture the cutoff. Until the DA
                    // layer reports `Synced` at least once, skip-advance the current slot
                    // unconditionally — this drains the channel so the runner can keep
                    // producing.
                    if matches!(resync_state, ResyncState::AwaitingSync) {
                        match self.da_sync_state.status() {
                            SyncStatus::Syncing { .. } => {
                                self.cursor.inc_next_height_to_receive_by(1);
                                continue;
                            }
                            SyncStatus::Synced { synced_da_height } => {
                                tracing::info!(
                                    %synced_da_height,
                                    "DA resync complete; recording skip cutoff for proof intake"
                                );
                                resync_state = ResyncState::SkipBelow(synced_da_height);
                                // fall through; this slot may still be below the cutoff.
                            }
                        }
                    }

                    if let ResyncState::SkipBelow(skip_height) = resync_state {
                        if stf_info.da_block_header().height() < skip_height {
                            // Skipped slots will never be proved, so advance
                            // the cursor manually.
                            self.cursor.inc_next_height_to_receive_by(1);
                            continue;
                        }
                    }

                    self.process_stf_info(stf_info).await?;
                }
            }
        }
        tracing::debug!("Aggregated proofs intake task has been completed");
        Ok(())
    }

    async fn process_stf_info(
        &mut self,
        stf_info: StateTransitionInfo<
            Ps::StateRoot,
            Ps::Witness,
            <Ps::DaService as DaService>::Spec,
        >,
    ) -> anyhow::Result<()> {
        let first_height_unproven = self.stf_info_receiver.next_height_to_receive();
        let received_slot_number = stf_info.slot_number();

        assert!(
            received_slot_number.get() >= first_height_unproven.get(),
            "Received slot {received_slot_number} is behind first unproven height {first_height_unproven}"
        );

        // We ensure that we're not trying to prove blocks that are being proven.
        // If that is not the case, we add the block to the queue.
        if first_height_unproven.saturating_add(self.proofs_to_create.current_proof_jump() as u64)
            <= stf_info.slot_number()
        {
            let block_header = stf_info.da_block_header().clone();
            // Save the transition for later proving. This is temporarily redundant
            // since we always just try to prove blocks right away (because we don't have fee
            // estimates for proving built out yet).
            self.proofs_to_create.append(BlockProofInfo {
                status: BlockProofStatus::Waiting(stf_info),
                header: block_header,
                // TODO(@preston-evans98): estimate public data size. This requires a new API on the `prover_service`.
                // <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/440>
                public_data_size: 0,
            });
        }

        if self.eager_proof_submission {
            self.proofs_to_create
                .oldest_mut()
                .prove_any_unproven_blocks(&*self.prover_service)
                .await;
        }

        let num_proofs_to_create = self.proofs_to_create.current_proof_jump();

        // If we've covered enough blocks for the aggregate proof, hand the
        // window off to the aggregator task.
        if num_proofs_to_create >= self.aggregated_proof_block_jump.get() {
            self.proofs_to_create.close_newest_proof();
            let metadata = self.proofs_to_create.take_oldest();
            let window_size = num_proofs_to_create as u64;

            // Capacity is `max_number_of_aggregated_proofs_in_memory`: blocks
            // here only once the aggregator has one window in flight AND that
            // many windows buffered. While blocked, intake stops draining the
            // STF-info mpsc, which propagates back-pressure upstream to the
            // runner exactly as in the pre-pipelining design.
            self.metadata_tx
                .send((metadata, window_size))
                .await
                .map_err(|_| {
                    anyhow::anyhow!("Aggregator task has terminated; cannot send metadata")
                })?;
        }

        let pending_agg_metadata = self.metadata_tx.max_capacity() - self.metadata_tx.capacity();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(super::metrics::ZkProofManagerMetrics {
                proving_lag: received_slot_number.get() - first_height_unproven.get(),
                proofs_to_create: self.proofs_to_create.current_proof_jump(),
                slot_number: received_slot_number.get(),
                pending_agg_metadata,
            });
        });

        Ok(())
    }
}

struct AggregatorTask<Ps: ProverService> {
    prover_service: Arc<Ps>,
    proof_sender: Box<dyn ProofSender>,
    backoff_policy: ExponentialBuilder,
    metadata_rx: mpsc::Receiver<(AggregateProofMetadata<Ps>, u64)>,
    cursor: CursorHandle,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
    shutdown_sender: tokio::sync::watch::Sender<()>,
}

impl<Ps: ProverService> AggregatorTask<Ps>
where
    Ps::DaService: DaService<Error = anyhow::Error>,
{
    async fn run(mut self) -> anyhow::Result<()> {
        loop {
            let (metadata, window_size) =
                match future_or_shutdown(self.metadata_rx.recv(), &self.shutdown_receiver).await {
                    FutureOrShutdownOutput::Shutdown => {
                        tracing::info!("Shutting down aggregator task...");
                        break;
                    }
                    FutureOrShutdownOutput::Output(None) => {
                        // Intake side dropped its sender (clean shutdown or
                        // intake error).
                        tracing::debug!("Intake task closed metadata channel; aggregator exiting");
                        break;
                    }
                    FutureOrShutdownOutput::Output(Some(item)) => item,
                };

            let status = self.proof_sender.proof_blob_sender_status().await?;
            if status.is_busy() {
                tracing::error!(
                    in_flight = status.in_flight,
                    max_concurrent_proof_blobs = status.max_concurrent,
                    "The zk proof blob sender is busy, which means proofs are not being confirmed by the rollup on time. Triggering shutdown."
                );
                if self.shutdown_sender.send(()).is_err() {
                    tracing::error!("Failed to send primary shutdown signal.");
                }
                break;
            }

            let proving_start = std::time::Instant::now();
            let agg_proof = create_aggregate_proof_with_retries(
                metadata,
                &*self.prover_service,
                &self.backoff_policy,
            )
            .await?;
            let aggregation_duration = proving_start.elapsed();

            sov_metrics::track_metrics(|tracker| {
                tracker.submit(super::metrics::ZkAggregatedProofMetrics {
                    aggregation_duration_ms: aggregation_duration.as_millis(),
                });
            });

            tracing::debug!(
                bytes = agg_proof.raw_aggregated_proof.len(),
                "Sending aggregated proof to DA"
            );

            self.proof_sender
                .publish_proof_blob_with_metadata(agg_proof)
                .await?;

            self.cursor.inc_next_height_to_receive_by(window_size);
        }
        tracing::debug!("Aggregator task has been completed");
        Ok(())
    }
}

async fn create_aggregate_proof_with_retries<Ps: ProverService>(
    mut metadata: AggregateProofMetadata<Ps>,
    prover_service: &Ps,
    backoff_policy: &ExponentialBuilder,
) -> anyhow::Result<SerializedAggregatedProof> {
    let mut attempt_num = 1u32;
    let mut backoff_iter = backoff_policy.build();

    loop {
        let maybe_backoff_duration = backoff_iter.next();

        match metadata.prove(prover_service).await {
            Ok(proof) => return Ok(proof),
            Err((returned_metadata, error)) => {
                let error_message = format!("Failed to generate aggregate proof: {error}");

                if error_message.contains("Elf parse error") {
                    // NOTE We exit early on this error since it means the we've failed to find/parse
                    // the zk circuit, and there's no recovering from that.
                    tracing::error!("Fatal error: {error_message}");
                    tracing::error!("Please check your zk circuit ELF file was built correctly!");
                    anyhow::bail!(error)
                };

                if error_message.contains("Outer network proving") {
                    tracing::error!("Fatal error: {error_message}");
                    anyhow::bail!(error)
                };

                tracing::error!(error_message);
                match maybe_backoff_duration {
                    None => {
                        tracing::warn!("Maximum number of retries exhausted - exiting");
                        anyhow::bail!(error)
                    }
                    Some(duration) => {
                        tracing::info!("Retrying generation of aggregate proof in {}s, attempt {attempt_num} of {}...", duration.as_secs(), BACKOFF_POLICY_MAX_NUM_RETRIES);
                        attempt_num += 1;
                        sleep(duration).await;
                        metadata = returned_metadata;
                        continue;
                    }
                }
            }
        }
    }
}
