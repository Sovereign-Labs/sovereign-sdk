use std::num::NonZero;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use backon::{BackoffBuilder, ExponentialBuilder};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use sov_rollup_interface::stf::ProofSender;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use types::{BlockProofInfo, BlockProofStatus, UnAggregatedProofList};

use self::types::AggregateProofMetadata;
use super::StateTransitionInfo;
use crate::processes::{ProverService, Receiver};

mod types;

const BACKOFF_POLICY_MIN_DELAY: u64 = 1;
const BACKOFF_POLICY_MAX_DELAY: u64 = 60;
const BACKOFF_POLICY_MAX_NUM_RETRIES: usize = 5;

/// Shared status for observing aggregate-proof progress from outside the proof manager task.
#[derive(Debug, Default)]
pub struct ZkProofManagerStatus {
    blocks_until_next_aggregate_proof: AtomicUsize,
}

impl ZkProofManagerStatus {
    /// Returns how many more blocks are needed before the next aggregate proof is ready.
    pub fn blocks_until_next_aggregate_proof(&self) -> usize {
        self.blocks_until_next_aggregate_proof
            .load(Ordering::SeqCst)
    }

    pub(crate) fn set_blocks_until_next_aggregate_proof(&self, value: usize) {
        self.blocks_until_next_aggregate_proof
            .store(value, Ordering::SeqCst);
    }
}

struct ProofProgress<Ps: ProverService> {
    proofs_to_create: UnAggregatedProofList<Ps>,
    aggregated_proof_block_jump: NonZero<usize>,
    status: Option<Arc<ZkProofManagerStatus>>,
    aggregate_proof_in_flight: bool,
}

impl<Ps: ProverService> ProofProgress<Ps> {
    fn new(
        aggregated_proof_block_jump: NonZero<usize>,
        status: Option<Arc<ZkProofManagerStatus>>,
    ) -> Self {
        let progress = Self {
            proofs_to_create: UnAggregatedProofList::new(),
            aggregated_proof_block_jump,
            status,
            aggregate_proof_in_flight: false,
        };
        progress.publish_status();
        progress
    }

    fn blocks_until_next_aggregate_proof(&self) -> usize {
        if self.aggregate_proof_in_flight {
            return 0;
        }

        self.aggregated_proof_block_jump
            .get()
            .saturating_sub(self.proofs_to_create.current_proof_jump())
    }

    fn current_proof_jump(&self) -> usize {
        self.proofs_to_create.current_proof_jump()
    }

    fn oldest_mut(&mut self) -> &mut AggregateProofMetadata<Ps> {
        self.proofs_to_create.oldest_mut()
    }

    fn append(&mut self, block: BlockProofInfo<Ps>) {
        self.proofs_to_create.append(block);
        self.publish_status();
    }

    fn should_create_aggregate_proof(&self) -> bool {
        self.current_proof_jump() >= self.aggregated_proof_block_jump.get()
    }

    fn begin_aggregate_proof_creation(&mut self) -> AggregateProofMetadata<Ps> {
        self.aggregate_proof_in_flight = true;
        self.proofs_to_create.close_newest_proof();
        let metadata = self.proofs_to_create.take_oldest();
        self.publish_status();
        metadata
    }

    fn finish_aggregate_proof_creation(&mut self) {
        self.aggregate_proof_in_flight = false;
        self.publish_status();
    }

    fn publish_status(&self) {
        if let Some(status) = &self.status {
            status.set_blocks_until_next_aggregate_proof(self.blocks_until_next_aggregate_proof());
        }
    }
}

/// Manages the lifecycle of the `AggregatedProof`.
#[allow(clippy::type_complexity)]
pub struct ZkProofManager<Ps: ProverService> {
    prover_service: Ps,
    proof_progress: ProofProgress<Ps>,
    proof_sender: Box<dyn ProofSender>,
    backoff_policy: ExponentialBuilder,
    genesis_state_root: Ps::StateRoot,
    stf_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
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
        proof_sender: Box<dyn ProofSender>,
        genesis_state_root: Ps::StateRoot,
        stf_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        status: Option<Arc<ZkProofManagerStatus>>,
    ) -> Self {
        Self {
            prover_service,
            proof_progress: ProofProgress::new(aggregated_proof_block_jump, status),
            proof_sender,
            backoff_policy: ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(BACKOFF_POLICY_MIN_DELAY))
                .with_max_delay(Duration::from_secs(BACKOFF_POLICY_MAX_DELAY))
                .with_max_times(BACKOFF_POLICY_MAX_NUM_RETRIES),
            genesis_state_root,
            stf_info_receiver,
            shutdown_receiver,
        }
    }

    /// Returns how many more blocks are needed before the next aggregate proof is ready.
    pub fn blocks_until_next_aggregate_proof(&self) -> usize {
        self.proof_progress.blocks_until_next_aggregate_proof()
    }

    async fn create_aggregate_proof_with_retries(
        &self,
        mut metadata: AggregateProofMetadata<Ps>,
        prover_service: &Ps,
        genesis_state_root: &Ps::StateRoot,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        let mut attempt_num = 1u32;
        let mut backoff_iter = self.backoff_policy.build();

        loop {
            let maybe_backoff_duration = backoff_iter.next();

            match metadata.prove(prover_service, genesis_state_root).await {
                Ok(proof) => return Ok(proof),
                Err((returned_metadata, error)) => {
                    let error_message = format!("Failed to generate aggregate proof: {error}");

                    if error_message.contains("Elf parse error") {
                        // NOTE We exit early on this error since it means the we've failed to find/parse
                        // the zk circuit, and there's no recovering from that.
                        tracing::error!("Fatal error: {error_message}");
                        tracing::error!(
                            "Please check your zk circuit ELF file was built correctly!"
                        );
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

    /// Starts a background task for `AggregatedProof` generation.
    pub async fn post_aggregated_proof_to_da_in_background(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            tracing::info!("Spawning an aggregated proof posting background task");
            if let Err(e) = self.post_aggregated_proof_to_da_when_ready().await {
                tracing::error!(error = ?e, "Failed to post aggregated proof to DA");
            }
        })
    }

    /// Attempts to generate an `AggregatedProof` and then posts it to DA.
    /// The proof is created only when there are enough of inner proofs in the `ProverService` queue.
    async fn post_aggregated_proof_to_da_when_ready(mut self) -> anyhow::Result<()> {
        loop {
            match future_or_shutdown(self.stf_info_receiver.read_next(), &self.shutdown_receiver)
                .await
            {
                FutureOrShutdownOutput::Shutdown => {
                    tracing::info!("Shutting down aggregated proof posting task...");
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
                        slot_number = %stf_info.slot_number,
                        block_header = %stf_info.da_block_header().display(),
                        "Received STF info");

                    self.process_stf_info(stf_info).await?;
                }
            }
        }
        tracing::debug!("Aggregated proofs posting task has been completed");
        Ok(())
    }

    /// Processes current STF info and optionally published aggregated proof to DA.
    async fn process_stf_info(
        &mut self,
        stf_info: StateTransitionInfo<
            Ps::StateRoot,
            Ps::Witness,
            <Ps::DaService as DaService>::Spec,
        >,
    ) -> anyhow::Result<()> {
        let first_height_unproven = self.stf_info_receiver.next_height_to_receive();

        let prover_service = &self.prover_service;

        // We ensure that we're not trying to prove blocks that are being proven.
        // If that is not the case, we add the block to the queue.
        if first_height_unproven.saturating_add(self.proof_progress.current_proof_jump() as u64)
            <= stf_info.slot_number
        {
            let block_hash = stf_info.da_block_header().hash();
            // Save the transition for later proving. This is temporarily redundant
            // since we always just try to prove blocks right away (because we don't have fee
            // estimates for proving built out yet).
            self.proof_progress.append(BlockProofInfo {
                status: BlockProofStatus::Waiting(stf_info),
                hash: block_hash,
                // TODO(@preston-evans98): estimate public data size. This requires a new API on the `prover_service`.
                // <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/440>
                public_data_size: 0,
            });
        }

        // Start proving the next block right away... for now.
        self.proof_progress
            .oldest_mut()
            .prove_any_unproven_blocks(prover_service)
            .await;

        let num_proofs_to_create = self.proof_progress.current_proof_jump();

        // If we've covered enough blocks for the aggregate proof, generate and submit it to DA
        if self.proof_progress.should_create_aggregate_proof() {
            let metadata = self.proof_progress.begin_aggregate_proof_creation();

            let agg_proof = self
                .create_aggregate_proof_with_retries(
                    metadata,
                    prover_service,
                    &self.genesis_state_root,
                )
                .await?;

            tracing::debug!(
                bytes = agg_proof.raw_aggregated_proof.len(),
                "Sending aggregated proof to DA"
            );

            self.proof_sender
                .publish_proof_blob_with_metadata(agg_proof)
                .await?;

            // Update the next height to receive
            self.stf_info_receiver
                .inc_next_height_to_receive_by(num_proofs_to_create as u64);
            self.proof_progress.finish_aggregate_proof_creation();
        }
        Ok(())
    }
}
