use std::collections::HashMap;

use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::StateTransitionPublicData;

use crate::processes::prover_service::block_proof::BlockProof;

pub(crate) struct SubmittedProofMetadata<Address, Da: DaSpec, StateRoot> {
    pub(crate) slot_number: SlotNumber,
    pub(crate) st: StateTransitionPublicData<Address, Da, StateRoot>,
}

pub(crate) enum NetworkProverStatus<Address, StateRoot, Da: DaSpec, Handle> {
    Submitted {
        handle: Handle,
        metadata: SubmittedProofMetadata<Address, Da, StateRoot>,
    },
    Proved(BlockProof<Address, Da, StateRoot>),
    Err(anyhow::Error),
}

pub(crate) struct NetworkProverState<Address, StateRoot, Da: DaSpec, Handle> {
    pub(crate) prover_status:
        HashMap<Da::SlotHash, NetworkProverStatus<Address, StateRoot, Da, Handle>>,
}

impl<Address, StateRoot, Da: DaSpec, Handle> NetworkProverState<Address, StateRoot, Da, Handle> {
    pub(crate) fn get_prover_status(
        &self,
        hash: &Da::SlotHash,
    ) -> Option<&NetworkProverStatus<Address, StateRoot, Da, Handle>> {
        self.prover_status.get(hash)
    }

    pub(crate) fn set_to_submitted(
        &mut self,
        hash: Da::SlotHash,
        handle: Handle,
        metadata: SubmittedProofMetadata<Address, Da, StateRoot>,
    ) {
        self.prover_status
            .insert(hash, NetworkProverStatus::Submitted { handle, metadata });
    }

    pub(crate) fn set_to_proved(
        &mut self,
        hash: Da::SlotHash,
        proof: BlockProof<Address, Da, StateRoot>,
    ) {
        self.prover_status
            .insert(hash, NetworkProverStatus::Proved(proof));
    }

    pub(crate) fn set_to_err(&mut self, hash: Da::SlotHash, err: anyhow::Error) {
        self.prover_status
            .insert(hash, NetworkProverStatus::Err(err));
    }

    pub(crate) fn remove(
        &mut self,
        hash: &Da::SlotHash,
    ) -> Option<NetworkProverStatus<Address, StateRoot, Da, Handle>> {
        self.prover_status.remove(hash)
    }
}
