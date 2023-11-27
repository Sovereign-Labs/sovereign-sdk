mod prover;
use std::sync::Arc;

use async_trait::async_trait;
use prover::Prover;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;

use super::{Hash, ProverService, ProverServiceError};
use crate::verifier::StateTransitionVerifier;
use crate::{
    ProofGenConfig, ProofProcessingStatus, ProofSubmissionStatus, RollupProverConfig,
    StateTransitionData,
};

/// Prover service that generates proofs in parallel.
pub struct ParallelProverService<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]>,
    Witness: Serialize + DeserializeOwned,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
{
    vm: Vm,
    prover_config: Arc<ProofGenConfig<V, Da, Vm>>,
    zk_storage: V::PreState,
    prover_state: Prover<StateRoot, Witness, Da>,
}

impl<StateRoot, Witness, Da, Vm, V> ParallelProverService<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
    V::PreState: Clone + Send + Sync,
{
    /// Creates a new prover.
    pub fn new(
        vm: Vm,
        zk_stf: V,
        da_verifier: Da::Verifier,
        config: RollupProverConfig,
        zk_storage: V::PreState,
        num_threads: usize,
    ) -> Self {
        let stf_verifier =
            StateTransitionVerifier::<V, Da::Verifier, Vm::Guest>::new(zk_stf, da_verifier);

        let config: ProofGenConfig<V, Da, Vm> = match config {
            RollupProverConfig::Skip => ProofGenConfig::Skip,
            RollupProverConfig::Simulate => ProofGenConfig::Simulate(stf_verifier),
            RollupProverConfig::Execute => ProofGenConfig::Execute,
            RollupProverConfig::Prove => ProofGenConfig::Prover,
        };

        let prover_config = Arc::new(config);

        Self {
            vm,
            prover_config,
            prover_state: Prover::new(num_threads),
            zk_storage,
        }
    }
}

#[async_trait]
impl<StateRoot, Witness, Da, Vm, V> ProverService
    for ParallelProverService<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
    Da: DaService,
    Vm: ZkvmHost + 'static,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync + 'static,
    V::PreState: Clone + Send + Sync,
{
    type StateRoot = StateRoot;

    type Witness = Witness;

    type DaService = Da;

    async fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    ) {
        self.prover_state.submit_witness(state_transition_data);
    }

    async fn prove(
        &self,
        block_header_hash: Hash,
    ) -> Result<ProofProcessingStatus, ProverServiceError> {
        let vm = self.vm.clone();
        let zk_storage = self.zk_storage.clone();

        self.prover_state.start_proving(
            block_header_hash,
            self.prover_config.clone(),
            vm,
            zk_storage,
        )
    }

    async fn send_proof_to_da(
        &self,
        block_header_hash: Hash,
    ) -> Result<ProofSubmissionStatus, anyhow::Error> {
        self.prover_state
            .get_proof_submission_status_and_remove_on_success(block_header_hash)
    }
}
