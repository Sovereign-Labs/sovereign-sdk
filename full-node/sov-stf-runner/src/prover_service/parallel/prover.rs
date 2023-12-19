use std::ops::Deref;
use std::sync::{Arc, RwLock};

use super::prover_manager::{ProverManager, ProverStatus};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::{Proof, ZkvmHost};

use super::ProverServiceError;
use crate::{
    ProofGenConfig, ProofProcessingStatus, ProofSubmissionStatus, StateTransitionData,
    WitnessSubmissionStatus,
};

// A prover that generates proofs in parallel using a thread pool. If the pool is saturated,
// the prover will reject new jobs.
pub(crate) struct Prover<StateRoot, Witness, Da: DaService> {
    prover_manager: Arc<RwLock<ProverManager<StateRoot, Witness, Da::Spec>>>,
    num_threads: usize,
    pool: rayon::ThreadPool,
}

impl<StateRoot, Witness, Da> Prover<StateRoot, Witness, Da>
where
    Da: DaService,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    pub(crate) fn new(num_threads: usize, jump: u64) -> Self {
        Self {
            num_threads,
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(num_threads)
                .build()
                .unwrap(),

            prover_manager: Arc::new(RwLock::new(ProverManager::new(jump))),
        }
    }

    pub(crate) fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<StateRoot, Witness, Da::Spec>,
    ) -> WitnessSubmissionStatus {
        let header_hash = state_transition_data.da_block_header.hash();
        let height = state_transition_data.da_block_header.height();
        self.prover_manager.write().unwrap().submit_witness(
            height,
            header_hash,
            state_transition_data,
        )
    }

    pub(crate) fn start_proving<Vm, V>(
        &self,
        block_header_hash: <Da::Spec as DaSpec>::SlotHash,
        config: Arc<ProofGenConfig<V, Da, Vm>>,
        mut vm: Vm,
        zk_storage: V::PreState,
    ) -> Result<ProofProcessingStatus, ProverServiceError>
    where
        Vm: ZkvmHost + 'static,
        V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync + 'static,
        V::PreState: Send + Sync + 'static,
    {
        let prover_manager_clone = self.prover_manager.clone();
        let mut prover_manager = self.prover_manager.write().expect("Lock was poisoned");

        let (prover_status, state_transition_data) = prover_manager
            .remove(&block_header_hash)
            .ok_or_else(|| anyhow::anyhow!("Missing witness for block: {:?}", block_header_hash))?;

        match prover_status {
            ProverStatus::WitnessSubmitted => {
                let start_prover = prover_manager.inc_task_count_if_not_busy(self.num_threads);

                // Initiate a new proving job only if the prover is not busy.
                if start_prover {
                    vm.add_hint(state_transition_data);

                    prover_manager.set_to_proving(block_header_hash.clone());

                    self.pool.spawn(move || {
                        tracing::info_span!("guest_execution").in_scope(|| {
                            let proof = make_proof(vm, config, zk_storage);

                            let mut prover_state =
                                prover_manager_clone.write().expect("Lock was poisoned");

                            prover_state.set_to_proved(block_header_hash, proof);
                            prover_state.dec_task_count();
                        })
                    });

                    Ok(ProofProcessingStatus::ProvingInProgress)
                } else {
                    Ok(ProofProcessingStatus::Busy)
                }
            }
            ProverStatus::ProvingInProgress => Err(anyhow::anyhow!(
                "Proof generation for {:?} still in progress",
                block_header_hash
            )
            .into()),
            ProverStatus::Proved(_) => Err(anyhow::anyhow!(
                "Witness for block_header_hash {:?}, submitted multiple times.",
                block_header_hash,
            )
            .into()),
            ProverStatus::Err(e) => Err(e.into()),
        }
    }

    pub(crate) fn get_proof_submission_status_and_remove_on_success(
        &self,
        block_header_hash: <Da::Spec as DaSpec>::SlotHash,
    ) -> Result<ProofSubmissionStatus, anyhow::Error> {
        let mut prover_manager = self.prover_manager.write().unwrap();
        let status = prover_manager.get_prover_status(&block_header_hash.clone());

        match status {
            Some(ProverStatus::ProvingInProgress) => {
                Ok(ProofSubmissionStatus::ProofGenerationInProgress)
            }
            Some(ProverStatus::Proved(_)) => {
                //TODO
                //prover_manager.remove(&block_header_hash);
                Ok(ProofSubmissionStatus::Success)
            }
            Some(ProverStatus::WitnessSubmitted) => Err(anyhow::anyhow!(
                "Witness for {:?} was submitted, but the proof generation is not triggered.",
                block_header_hash
            )),
            Some(ProverStatus::Err(e)) => Err(anyhow::anyhow!(e.to_string())),
            None => Err(anyhow::anyhow!(
                "Missing witness for: {:?}",
                block_header_hash
            )),
        }
    }
}

fn make_proof<V, Vm, Da>(
    mut vm: Vm,
    config: Arc<ProofGenConfig<V, Da, Vm>>,
    zk_storage: V::PreState,
) -> Result<Proof, anyhow::Error>
where
    Da: DaService,
    Vm: ZkvmHost + 'static,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync + 'static,
    V::PreState: Send + Sync + 'static,
{
    match config.deref() {
        ProofGenConfig::Skip => Ok(Proof::Empty),
        ProofGenConfig::Simulate(verifier) => verifier
            .run_block(vm.simulate_with_hints(), zk_storage)
            .map(|_| Proof::Empty)
            .map_err(|e| anyhow::anyhow!("Guest execution must succeed but failed with {:?}", e)),
        ProofGenConfig::Execute => vm.run(false),
        ProofGenConfig::Prover => vm.run(true),
    }
}
