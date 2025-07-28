use std::collections::HashMap;

use sov_rollup_interface::da::DaSpec;

use crate::processes::prover_service::block_proof::BlockProof;

pub(crate) enum ProverStatus<Address, StateRoot, Da: DaSpec> {
    ProvingInProgress,
    Proved(BlockProof<Address, Da, StateRoot>),
    Err(anyhow::Error),
}

pub(crate) struct ProverState<Address, StateRoot, Da: DaSpec> {
    pub(crate) prover_status: HashMap<Da::SlotHash, ProverStatus<Address, StateRoot, Da>>,
    pub(crate) pending_tasks_count: usize,
}

impl<Address, StateRoot, Da: DaSpec> ProverState<Address, StateRoot, Da> {
    pub(crate) fn remove(
        &mut self,
        hash: &Da::SlotHash,
    ) -> Option<ProverStatus<Address, StateRoot, Da>> {
        self.prover_status.remove(hash)
    }

    pub(crate) fn set_to_proving(
        &mut self,
        hash: Da::SlotHash,
    ) -> Option<ProverStatus<Address, StateRoot, Da>> {
        self.prover_status
            .insert(hash, ProverStatus::ProvingInProgress)
    }

    pub(crate) fn set_to_proved(
        &mut self,
        hash: Da::SlotHash,
        proof: anyhow::Result<BlockProof<Address, Da, StateRoot>>,
    ) -> Option<ProverStatus<Address, StateRoot, Da>> {
        match proof {
            Ok(p) => self.prover_status.insert(hash, ProverStatus::Proved(p)),
            Err(e) => self.prover_status.insert(hash, ProverStatus::Err(e)),
        }
    }

    pub(crate) fn get_prover_status(
        &self,
        hash: &Da::SlotHash,
    ) -> Option<&ProverStatus<Address, StateRoot, Da>> {
        self.prover_status.get(hash)
    }

    pub(crate) fn inc_task_count_if_not_busy(&mut self, num_threads: usize) -> bool {
        if self.pending_tasks_count >= num_threads {
            return false;
        }

        self.pending_tasks_count += 1;
        true
    }

    pub(crate) fn dec_task_count(&mut self) {
        assert!(self.pending_tasks_count > 0);
        self.pending_tasks_count -= 1;
    }
}
