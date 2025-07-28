//! Processes responsible for creating different kind of proofs.
mod op_manager;
mod prover_service;
mod stf_info_manager;
mod zk_manager;
use std::num::NonZero;

use op_manager::attestations::AttestationsManager;
pub use prover_service::*;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::optimistic::BondingProofService;
use sov_rollup_interface::stf::ProofSender;
pub use stf_info_manager::*;
use tokio::sync::watch;
use tokio::task::JoinHandle;
pub use zk_manager::*;

/// Starts a process that generates aggregated proofs in the background.
pub async fn start_zk_workflow_in_background<Ps>(
    prover_service: Ps,
    aggregated_proof_block_jump: NonZero<usize>,
    proof_sender: Box<dyn ProofSender>,
    genesis_state_root: Ps::StateRoot,
    stf_info_receiver: Receiver<Ps::StateRoot, Ps::Witness, <Ps::DaService as DaService>::Spec>,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
) -> anyhow::Result<JoinHandle<()>>
where
    Ps: ProverService,
    Ps::DaService: DaService<Error = anyhow::Error>,
{
    Ok(ZkProofManager::new(
        prover_service,
        aggregated_proof_block_jump,
        proof_sender,
        genesis_state_root,
        stf_info_receiver,
        shutdown_receiver,
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
