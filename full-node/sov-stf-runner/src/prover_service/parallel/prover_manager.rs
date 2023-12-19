use crate::{ProofSubmissionStatus, StateTransitionData, WitnessSubmissionStatus};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::Proof;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

pub(crate) enum ProverStatus {
    WitnessSubmitted,
    ProvingInProgress,
    Proved(Proof),
    Err(anyhow::Error),
}

struct ProverState<StateRoot, Witness, Da: DaSpec> {
    prover_status: HashMap<Da::SlotHash, ProverStatus>,
    witness: HashMap<Da::SlotHash, StateTransitionData<StateRoot, Witness, Da>>,
    pending_tasks_count: usize,
}

impl<StateRoot, Witness, Da: DaSpec> ProverState<StateRoot, Witness, Da> {
    fn remove(&mut self, hash: &Da::SlotHash) -> Option<ProverStatus> {
        self.prover_status.remove(hash)
    }

    fn remove_witness(
        &mut self,
        hash: &Da::SlotHash,
    ) -> Option<StateTransitionData<StateRoot, Witness, Da>> {
        self.witness.remove(hash)
    }

    fn set_to_proving(&mut self, hash: Da::SlotHash) -> Option<ProverStatus> {
        self.prover_status
            .insert(hash, ProverStatus::ProvingInProgress)
    }

    fn set_to_proved(
        &mut self,
        hash: Da::SlotHash,
        proof: Result<Proof, anyhow::Error>,
    ) -> Option<ProverStatus> {
        match proof {
            Ok(p) => self.prover_status.insert(hash, ProverStatus::Proved(p)),
            Err(e) => self.prover_status.insert(hash, ProverStatus::Err(e)),
        }
    }

    fn get_prover_status(&self, hash: &Da::SlotHash) -> Option<&ProverStatus> {
        self.prover_status.get(hash)
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

#[derive(Default)]
struct AggregatedProofInfo<SlotHash> {
    height_to_slot_hash: HashMap<u64, SlotHash>,
    start_height: u64,
    jump: u64,
}

impl<SlotHash> AggregatedProofInfo<SlotHash> {}

pub(crate) struct ProverManager<StateRoot, Witness, Da: DaSpec> {
    prover_state: ProverState<StateRoot, Witness, Da>,
    aggregated_proof_info: AggregatedProofInfo<Da::SlotHash>,
}

impl<StateRoot, Witness, Da: DaSpec> ProverManager<StateRoot, Witness, Da> {
    pub(crate) fn new(jump: u64) -> Self {
        Self {
            prover_state: ProverState {
                prover_status: Default::default(),
                pending_tasks_count: Default::default(),
                witness: Default::default(),
            },
            aggregated_proof_info: AggregatedProofInfo {
                height_to_slot_hash: Default::default(),
                start_height: 0,
                jump,
            },
        }
    }

    pub(crate) fn set_to_proving(&mut self, hash: Da::SlotHash) -> Option<ProverStatus> {
        self.prover_state.set_to_proving(hash)
    }

    pub(crate) fn set_to_proved(
        &mut self,
        hash: Da::SlotHash,
        proof: Result<Proof, anyhow::Error>,
    ) -> Option<ProverStatus> {
        self.prover_state.set_to_proved(hash, proof)
    }

    pub(crate) fn inc_task_count_if_not_busy(&mut self, num_threads: usize) -> bool {
        self.prover_state.inc_task_count_if_not_busy(num_threads)
    }

    pub(crate) fn dec_task_count(&mut self) {
        self.prover_state.dec_task_count()
    }

    pub(crate) fn get_witness(
        &mut self,
        hash: &Da::SlotHash,
    ) -> &StateTransitionData<StateRoot, Witness, Da> {
        self.prover_state.witness.get(hash).unwrap()
    }

    pub(crate) fn submit_witness(
        &mut self,
        height: u64,
        header_hash: Da::SlotHash,
        state_transition_data: StateTransitionData<StateRoot, Witness, Da>,
    ) -> WitnessSubmissionStatus {
        let entry = self.prover_state.prover_status.entry(header_hash.clone());
        let data = ProverStatus::WitnessSubmitted;

        match entry {
            Entry::Occupied(_) => WitnessSubmissionStatus::WitnessExist,
            Entry::Vacant(v) => {
                v.insert(data);
                // TODO assert first insertion
                self.aggregated_proof_info
                    .height_to_slot_hash
                    .insert(height, header_hash.clone());

                self.prover_state
                    .witness
                    .insert(header_hash, state_transition_data);

                WitnessSubmissionStatus::SubmittedForProving
            }
        }
    }

    // TODO change name
    pub(crate) fn remove(
        &mut self,
        hash: &Da::SlotHash,
    ) -> Option<(ProverStatus, StateTransitionData<StateRoot, Witness, Da>)> {
        let status = self.prover_state.remove(hash)?;
        let witness = self.prover_state.remove_witness(hash)?;
        Some((status, witness))
    }

    pub(crate) fn get_prover_status(&mut self, hash: &Da::SlotHash) -> Option<&ProverStatus> {
        self.prover_state.get_prover_status(hash)
    }

    fn get_aggregated_proof(&mut self) -> Result<ProofSubmissionStatus, anyhow::Error> {
        let jump = self.aggregated_proof_info.jump;
        let start_height = self.aggregated_proof_info.start_height;

        let mut proofs_data = Vec::default();

        for height in start_height..start_height + jump {
            let hash = self
                .aggregated_proof_info
                .height_to_slot_hash
                .get(&height)
                .unwrap();

            let state = self.prover_state.get_prover_status(hash).unwrap();
            match state {
                ProverStatus::WitnessSubmitted => {
                    return Err(anyhow::anyhow!(
                    "Witness for {:?} was submitted, but the proof generation is not triggered.",
                    hash
                ))
                }
                ProverStatus::ProvingInProgress => {
                    return Ok(ProofSubmissionStatus::ProofGenerationInProgress)
                }
                ProverStatus::Proved(proof) => proofs_data.push(proof),
                ProverStatus::Err(e) => return Err(anyhow::anyhow!(e.to_string())),
            }
        }

        todo!()
    }
}

struct AggregatedProofWitness<StateRoot, SlotHash> {
    proof: Proof,
    pre_state: StateRoot,
    post_state_root: StateRoot,
    da_block_hash: SlotHash,
    height: u64,
}

struct AggregatedProofPublicInput {
    initial_state: u64,
    final_state_root: u64,
    initial_height: u64,
    final_height: u64,
}

struct AggrgatedProof {}
