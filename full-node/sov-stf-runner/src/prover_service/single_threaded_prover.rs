use super::Hash;
use super::{ProverService, ProverServiceError};

use crate::verifier::StateTransitionVerifier;
use crate::StateTransitionData;
use crate::{ProofGenConfig, Prover};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use std::{collections::HashMap, sync::Mutex};

/// The possible configurations of the prover.
pub enum RollupProverConfig {
    /// Run the rollup verification logic inside the current process
    Simulate,
    /// Run the rollup verifier in a zkVM executor
    Execute,
    /// Run the rollup verifier and create a SNARK of execution
    Prove,
}

pub struct SimpleProver<StateRoot, Witness, Da, Vm, V, Stf>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]>,
    Witness: Serialize + DeserializeOwned,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
    Stf: StateTransitionFunction<Vm, Da::Spec, Condition = <Da::Spec as DaSpec>::ValidityCondition>,
{
    prover: Mutex<Option<Prover<V, Da, Vm>>>,
    data: Mutex<HashMap<Hash, StateTransitionData<StateRoot, Witness, Da::Spec>>>,
    zk_storage: V::PreState,
    da_service: Da,
    stf: Stf,
}

impl<StateRoot, Witness, Da, Vm, V, Stf> SimpleProver<StateRoot, Witness, Da, Vm, V, Stf>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync,
    Witness: Serialize + DeserializeOwned + Send + Sync,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
    Stf: StateTransitionFunction<Vm, Da::Spec, Condition = <Da::Spec as DaSpec>::ValidityCondition>,
    V::PreState: Clone + Send + Sync,
{
    pub fn new(
        vm: Vm,
        v: V,
        da_v: Da::Verifier,
        config: ProofGenConfig<V, Da, Vm>,
        zk_storage: V::PreState,
        da_service: Da,
        stf: Stf,
    ) -> Self {
        let x = StateTransitionVerifier::<V, Da::Verifier, Vm::Guest>::new(v, da_v);

        let prover = Prover { vm, config };
        todo!()
    }
}

#[async_trait]
impl<StateRoot, Witness, Da, Vm, V, Stf> ProverService
    for SimpleProver<StateRoot, Witness, Da, Vm, V, Stf>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync,
    Witness: Serialize + DeserializeOwned + Send + Sync,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
    V::PreState: Clone + Send + Sync,
    Stf: StateTransitionFunction<Vm, Da::Spec, Condition = <Da::Spec as DaSpec>::ValidityCondition>
        + Send
        + Sync,
{
    type StateRoot = StateRoot;

    type Witness = Witness;

    type DaService = Da;

    fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    ) {
        let header_hash = state_transition_data.da_block_header.hash().into();
        self.data
            .lock()
            .expect("Lock was poisoned")
            .insert(header_hash, state_transition_data);
    }

    async fn prove(&self, block_header_hash: Hash) -> Result<(), ProverServiceError> {
        if let Some(Prover { vm, config }) = self.prover.lock().expect("Lock was poisoned").as_mut()
        {
            let transition_data = self
                .data
                .lock()
                .expect("Lock was poisoned")
                .remove(&block_header_hash)
                .unwrap();

            vm.add_hint(transition_data);

            tracing::info_span!("guest_execution").in_scope(|| match config {
                ProofGenConfig::Simulate(verifier) => verifier
                    .run_block(vm.simulate_with_hints(), self.zk_storage.clone())
                    .map_err(|e| {
                        anyhow::anyhow!("Guest execution must succeed but failed with {:?}", e)
                    })
                    .map(|_| ()),
                ProofGenConfig::Execute => vm.run(false),
                ProofGenConfig::Prover => vm.run(true),
            })?;
        }
        todo!()
    }
}
