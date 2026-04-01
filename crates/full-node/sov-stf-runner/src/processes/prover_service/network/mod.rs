mod prover;

use std::sync::Arc;

use async_trait::async_trait;
use borsh::BorshSerialize;
use prover::NetworkProver;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::CodeCommitmentHash;
use sov_rollup_interface::zk::{Zkvm, ZkvmGuest};

use super::{ProverService, ProverServiceError, Verifier};
use crate::processes::{ProofAggregationStatus, ProofProcessingStatus, StateTransitionInfo};

/// Prover service that submits proofs to a remote proving network.
///
/// Instead of generating proofs locally (like [`super::ParallelProverService`]),
/// this service uses the [`ZkvmNetwork`] trait to submit proof requests to a
/// remote proving service and polls for completion.
pub struct NetworkProverService<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
where
    Address: Serialize + DeserializeOwned,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]>,
    Witness: Serialize + DeserializeOwned,
    Da: DaService,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
{
    prover: NetworkProver<Address, StateRoot, Witness, Da, InnerVm, OuterVm>,
    verifier: Arc<Verifier<Da>>,
}

impl<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
    NetworkProverService<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
where
    Address:
        BorshSerialize + AsRef<[u8]> + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
    Da: DaService,
    InnerVm: Zkvm + 'static,
    OuterVm: Zkvm,
{
    /// Creates a new network prover service.
    pub fn new(
        inner_vm: InnerVm::Network,
        outer_vm: OuterVm::Network,
        da_verifier: Da::Verifier,
        code_commitment: CodeCommitmentHash,
        prover_address: Address,
        outer_proof_timeout: std::time::Duration,
    ) -> Self {
        let verifier = Arc::new(Verifier { da_verifier });

        Self {
            prover: NetworkProver::new(
                prover_address,
                inner_vm,
                outer_vm,
                code_commitment,
                outer_proof_timeout,
            ),
            verifier,
        }
    }
}

#[async_trait]
impl<Address, StateRoot, Witness, Da, InnerVm, OuterVm> ProverService
    for NetworkProverService<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
where
    Address:
        BorshSerialize + AsRef<[u8]> + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    StateRoot:
        BorshSerialize + Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
    Da: DaService,
    InnerVm: Zkvm + 'static,
    OuterVm: Zkvm + 'static,
{
    type StateRoot = StateRoot;
    type Witness = Witness;
    type DaService = Da;
    type Verifier = <OuterVm::Guest as ZkvmGuest>::Verifier;

    async fn prove(
        &self,
        state_transition_info: StateTransitionInfo<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    ) -> Result<
        ProofProcessingStatus<Self::StateRoot, Self::Witness, <Self::DaService as DaService>::Spec>,
        ProverServiceError,
    > {
        self.prover
            .start_proving(state_transition_info, &self.verifier)
            .await
    }

    async fn create_aggregated_proof(
        &self,
        block_header_hashes: &[<<Self::DaService as DaService>::Spec as DaSpec>::SlotHash],
        genesis_state_root: &Self::StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus> {
        self.prover
            .create_aggregated_proof(block_header_hashes, genesis_state_root)
            .await
    }
}
