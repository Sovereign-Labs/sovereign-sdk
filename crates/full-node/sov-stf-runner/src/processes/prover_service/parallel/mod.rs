mod prover;
mod state;
use std::sync::Arc;

use async_trait::async_trait;
use borsh::BorshSerialize;
use prover::Prover;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::CodeCommitment;
use sov_rollup_interface::zk::{Zkvm, ZkvmGuest};

use super::{ProverService, ProverServiceError, RollupProverConfigDiscriminants};
use crate::processes::{ProofAggregationStatus, ProofProcessingStatus, StateTransitionInfo};
pub(crate) struct Verifier<Da>
where
    Da: DaService,
{
    pub(crate) da_verifier: Da::Verifier,
}

/// Prover service that generates proofs in parallel.
pub struct ParallelProverService<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
where
    Address: Serialize + DeserializeOwned,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]>,
    Witness: Serialize + DeserializeOwned,
    Da: DaService,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
{
    inner_vm: InnerVm::Host,
    outer_vm: OuterVm::Host,
    prover_config: RollupProverConfigDiscriminants,

    prover_state: Prover<Address, StateRoot, Witness, Da>,

    verifier: Arc<Verifier<Da>>,
}

impl<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
    ParallelProverService<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
where
    Address:
        BorshSerialize + AsRef<[u8]> + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
    Da: DaService,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
{
    /// Creates a new prover.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        inner_vm: InnerVm::Host,
        outer_vm: OuterVm::Host,
        da_verifier: Da::Verifier,
        config: RollupProverConfigDiscriminants,
        num_threads: usize,
        code_commitment: CodeCommitment,
        prover_address: Address,
    ) -> Self {
        let verifier = Arc::new(Verifier { da_verifier });

        Self {
            inner_vm,
            outer_vm,
            prover_config: config,
            prover_state: Prover::new(prover_address, num_threads, code_commitment),
            verifier,
        }
    }

    /// Creates a new prover.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_default_workers(
        inner_vm: InnerVm::Host,
        outer_vm: OuterVm::Host,
        da_verifier: Da::Verifier,
        config: RollupProverConfigDiscriminants,
        code_commitment: CodeCommitment,
        prover_address: Address,
    ) -> Self {
        let num_cpus = num_cpus::get();
        assert!(num_cpus > 1, "Unable to create parallel prover service");

        Self::new(
            inner_vm,
            outer_vm,
            da_verifier,
            config,
            num_cpus - 1,
            code_commitment,
            prover_address,
        )
    }
}

#[async_trait]
impl<Address, StateRoot, Witness, Da, InnerVm, OuterVm> ProverService
    for ParallelProverService<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
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
        let inner_vm = self.inner_vm.clone();

        self.prover_state.start_proving::<InnerVm>(
            state_transition_info,
            self.prover_config,
            inner_vm,
            self.verifier.clone(),
        )
    }

    async fn create_aggregated_proof(
        &self,
        block_header_hashes: &[<<Self::DaService as DaService>::Spec as DaSpec>::SlotHash],
        genesis_state_root: &Self::StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus> {
        self.prover_state.create_aggregated_proof(
            self.outer_vm.clone(),
            block_header_hashes,
            genesis_state_root,
        )
    }
}
