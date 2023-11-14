use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;

use super::{Hash, ProverService, ProverServiceError};
use crate::verifier::StateTransitionVerifier;
use crate::{ProofGenConfig, Prover, RollupProverConfig, StateTransitionData};

/// Prover that blocks the current thread and creates a ZKP proof.
pub struct BlockingProver<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]>,
    Witness: Serialize + DeserializeOwned,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
{
    prover: Mutex<Option<Prover<V, Da, Vm>>>,
    #[allow(clippy::type_complexity)]
    data: Mutex<HashMap<Hash, StateTransitionData<StateRoot, Witness, Da::Spec>>>,
    zk_storage: V::PreState,
    _da_service: Da,
}

impl<StateRoot, Witness, Da, Vm, V> BlockingProver<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync,
    Witness: Serialize + DeserializeOwned + Send + Sync,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
    V::PreState: Clone + Send + Sync,
{
    /// TODO
    pub fn new(
        vm: Vm,
        v: V,
        da_v: Da::Verifier,
        config: Option<RollupProverConfig>,
        zk_storage: V::PreState,
        _da_service: Da,
    ) -> Self {
        let stf_verifier = StateTransitionVerifier::<V, Da::Verifier, Vm::Guest>::new(v, da_v);

        let config: ProofGenConfig<V, Da, Vm> = match config.unwrap() {
            RollupProverConfig::Simulate => ProofGenConfig::Simulate(stf_verifier),
            RollupProverConfig::Execute => ProofGenConfig::Execute,
            RollupProverConfig::Prove => ProofGenConfig::Prover,
        };

        let prover = Prover { vm, config };

        Self {
            prover: Mutex::new(Some(prover)),
            data: Mutex::new(HashMap::new()),
            zk_storage,
            _da_service,
        }
    }
}

#[async_trait]
impl<StateRoot, Witness, Da, Vm, V> ProverService for BlockingProver<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync,
    Witness: Serialize + DeserializeOwned + Send + Sync,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
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
        let header_hash = state_transition_data.da_block_header.hash().into();
        self.data
            .lock()
            .expect("Lock was poisoned")
            .insert(header_hash, state_transition_data);
    }

    async fn prove(&self, block_header_hash: Hash) -> Result<(), ProverServiceError> {
        if let Some(Prover { vm, config }) = self.prover.lock().expect("Lock was poisoned").as_mut()
        {
            let transition_data = {
                self.data
                    .lock()
                    .expect("Lock was poisoned")
                    .remove(&block_header_hash)
                    .unwrap()
            };

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

        Ok(())
    }
}
