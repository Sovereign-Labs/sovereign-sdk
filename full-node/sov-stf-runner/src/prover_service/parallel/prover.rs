use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, RwLock};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::{Proof, StateTransitionData, ZkvmHost};

use super::ProverServiceError;
use crate::{
    ProofGenConfig, ProofProcessingStatus, ProofSubmissionStatus, WitnessSubmissionStatus,
};

enum ProverStatus<StateRoot, Witness, Da: DaSpec> {
    WitnessSubmitted(StateTransitionData<StateRoot, Witness, Da>),
    ProvingInProgress,
    Proved(Proof),
    Err(anyhow::Error),
}

struct ProverState<StateRoot, Witness, Da: DaSpec> {
    prover_status: HashMap<Da::SlotHash, ProverStatus<StateRoot, Witness, Da>>,
    pending_tasks_count: usize,
}

impl<StateRoot, Witness, Da: DaSpec> ProverState<StateRoot, Witness, Da> {
    fn remove(&mut self, hash: &Da::SlotHash) -> Option<ProverStatus<StateRoot, Witness, Da>> {
        self.prover_status.remove(hash)
    }

    fn set_to_proving(
        &mut self,
        hash: Da::SlotHash,
    ) -> Option<ProverStatus<StateRoot, Witness, Da>> {
        self.prover_status
            .insert(hash, ProverStatus::ProvingInProgress)
    }

    fn set_to_proved(
        &mut self,
        hash: Da::SlotHash,
        proof: Result<Proof, anyhow::Error>,
    ) -> Option<ProverStatus<StateRoot, Witness, Da>> {
        match proof {
            Ok(p) => self.prover_status.insert(hash, ProverStatus::Proved(p)),
            Err(e) => self.prover_status.insert(hash, ProverStatus::Err(e)),
        }
    }

    fn get_prover_status(
        &self,
        hash: Da::SlotHash,
    ) -> Option<&ProverStatus<StateRoot, Witness, Da>> {
        self.prover_status.get(&hash)
    }

    fn inc_task_count_if_not_busy(&mut self, num_threads: usize) -> bool {
        if self.pending_tasks_count >= num_threads {
            return false;
        }

        self.pending_tasks_count += 1;
        true
    }

    fn dec_task_count(&mut self) {
        assert!(self.pending_tasks_count > 0);
        self.pending_tasks_count -= 1;
    }
}

// A prover that generates proofs in parallel using a thread pool. If the pool is saturated,
// the prover will reject new jobs.
pub(crate) struct Prover<StateRoot, Witness, Da: DaService> {
    prover_state: Arc<RwLock<ProverState<StateRoot, Witness, Da::Spec>>>,
    num_threads: usize,
    pool: rayon::ThreadPool,
    _aggregated_proof_block_jump: u64,
}

impl<StateRoot, Witness, Da> Prover<StateRoot, Witness, Da>
where
    Da: DaService,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    pub(crate) fn new(num_threads: usize, _aggregated_proof_block_jump: u64) -> Self {
        Self {
            num_threads,
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(num_threads)
                .build()
                .unwrap(),

            prover_state: Arc::new(RwLock::new(ProverState {
                prover_status: Default::default(),
                pending_tasks_count: Default::default(),
            })),
            _aggregated_proof_block_jump,
        }
    }

    pub(crate) fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<StateRoot, Witness, Da::Spec>,
    ) -> WitnessSubmissionStatus {
        let header_hash = state_transition_data.da_block_header.hash();
        let data = ProverStatus::WitnessSubmitted(state_transition_data);

        let mut prover_state = self.prover_state.write().expect("Lock was poisoned");
        let entry = prover_state.prover_status.entry(header_hash);

        match entry {
            Entry::Occupied(_) => WitnessSubmissionStatus::WitnessExist,
            Entry::Vacant(v) => {
                v.insert(data);
                WitnessSubmissionStatus::SubmittedForProving
            }
        }
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
        let prover_state_clone = self.prover_state.clone();
        let mut prover_state = self.prover_state.write().expect("Lock was poisoned");

        let prover_status = prover_state
            .remove(&block_header_hash)
            .ok_or_else(|| anyhow::anyhow!("Missing witness for block: {:?}", block_header_hash))?;

        match prover_status {
            ProverStatus::WitnessSubmitted(state_transition_data) => {
                let start_prover = prover_state.inc_task_count_if_not_busy(self.num_threads);

                // Initiate a new proving job only if the prover is not busy.
                if start_prover {
                    prover_state.set_to_proving(block_header_hash.clone());
                    vm.add_hint(state_transition_data);

                    self.pool.spawn(move || {
                        tracing::info_span!("guest_execution").in_scope(|| {
                            let proof = make_proof(vm, config, zk_storage);

                            let mut prover_state =
                                prover_state_clone.write().expect("Lock was poisoned");

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
        let mut prover_state = self.prover_state.write().unwrap();
        let status = prover_state.get_prover_status(block_header_hash.clone());

        match status {
            Some(ProverStatus::ProvingInProgress) => {
                Ok(ProofSubmissionStatus::ProofGenerationInProgress)
            }
            Some(ProverStatus::Proved(_)) => {
                prover_state.remove(&block_header_hash);
                Ok(ProofSubmissionStatus::Success)
            }
            Some(ProverStatus::WitnessSubmitted(_)) => Err(anyhow::anyhow!(
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
        ProofGenConfig::Skip => Ok(Proof::PublicInput(Vec::default())),
        ProofGenConfig::Simulate(verifier) => verifier
            .run_block(vm.simulate_with_hints(), zk_storage)
            .map(|_| Proof::PublicInput(Vec::default()))
            .map_err(|e| anyhow::anyhow!("Guest execution must succeed but failed with {:?}", e)),
        ProofGenConfig::Execute => vm.run(false),
        ProofGenConfig::Prover => vm.run(true),
    }
}
