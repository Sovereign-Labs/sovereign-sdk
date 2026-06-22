//! Processes responsible for creating different kind of proofs.
mod metrics;
mod op_manager;
mod prover_service;
mod stf_info_manager;
mod zk_manager;
use std::num::NonZero;
use std::sync::Arc;

use op_manager::attestations::AttestationsManager;
pub use prover_service::*;
use sov_rollup_full_node_interface::{DaSyncState, PrimaryShutdownController};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::optimistic::BondingProofService;
use sov_rollup_interface::stf::ProofSender;
pub use stf_info_manager::*;
use tokio::sync::watch;
use tokio::task::JoinHandle;
pub use zk_manager::*;

/// Starts a process that generates aggregated proofs in the background.
#[allow(clippy::too_many_arguments)]
pub async fn start_zk_workflow_in_background<Ps>(
    prover_service: Ps,
    aggregated_proof_block_jump: NonZero<usize>,
    eager_proof_submission: bool,
    max_number_of_aggregated_proofs_in_memory: NonZero<usize>,
    proof_sender: Box<dyn ProofSender>,
    stf_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
    da_sync_state: Arc<DaSyncState>,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
    shutdown_sender: PrimaryShutdownController,
    start_fresh_outer_proof_on_resync: bool,
) -> anyhow::Result<JoinHandle<()>>
where
    Ps: ProverService,
    Ps::DaService: DaService<Error = anyhow::Error>,
{
    Ok(ZkProofManager::new(
        prover_service,
        aggregated_proof_block_jump,
        eager_proof_submission,
        max_number_of_aggregated_proofs_in_memory,
        proof_sender,
        stf_info_receiver,
        da_sync_state,
        shutdown_receiver,
        shutdown_sender,
        start_fresh_outer_proof_on_resync,
    )
    .post_aggregated_proof_to_da_in_background()
    .await)
}

/// Starts the process that generates optimistic proofs in the background.
pub async fn start_op_workflow_in_background<Ps, Bps>(
    bonding_proof_service: Bps,
    proof_sender: Box<dyn ProofSender>,
    shutdown_receiver: watch::Receiver<()>,
    st_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
) -> anyhow::Result<JoinHandle<()>>
where
    Ps: ProverService,
    Ps::DaService: DaService<Error = anyhow::Error>,
    Bps: BondingProofService,
{
    Ok(AttestationsManager::new(
        st_info_receiver,
        bonding_proof_service,
        proof_sender,
        shutdown_receiver,
    )
    .post_attestation_to_da_in_background()
    .await)
}

/// Starts the operator workflow in the background.
pub async fn start_operator_workflow_in_background(
    mut shutdown_receiver: watch::Receiver<()>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let _ = shutdown_receiver.changed().await;
    })
}
