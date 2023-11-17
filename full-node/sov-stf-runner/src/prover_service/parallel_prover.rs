use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use super::{Hash, ProverService, ProverServiceError};
use crate::verifier::StateTransitionVerifier;
use crate::{ProofGenConfig, RollupProverConfig, StateTransitionData};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;

enum ProverState<StateRoot, Witness, Da: DaSpec> {
    WitnessSubmitted(StateTransitionData<StateRoot, Witness, Da>),
    Proving,
    Proved(Vec<u8>),
    Err(anyhow::Error),
}

/// TODO
pub struct ParallelProver<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]>,
    Witness: Serialize + DeserializeOwned,
    Da: DaService,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync,
{
    vm: Vm,
    prover_config: Option<Arc<ProofGenConfig<V, Da, Vm>>>,
    zk_storage: V::PreState,

    #[allow(clippy::type_complexity)]
    prover_state: Mutex<HashMap<Hash, ProverState<StateRoot, Witness, Da::Spec>>>,
}

impl<StateRoot, Witness, Da, Vm, V> ParallelProver<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync,
    Witness: Serialize + DeserializeOwned + Send + Sync,
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
        config: Option<RollupProverConfig>,
        zk_storage: V::PreState,
    ) -> Self {
        let prover_config = config.map(|config| {
            let stf_verifier =
                StateTransitionVerifier::<V, Da::Verifier, Vm::Guest>::new(zk_stf, da_verifier);

            let config: ProofGenConfig<V, Da, Vm> = match config {
                RollupProverConfig::Simulate => ProofGenConfig::Simulate(stf_verifier),
                RollupProverConfig::Execute => ProofGenConfig::Execute,
                RollupProverConfig::Prove => ProofGenConfig::Prover,
            };

            Arc::new(config)
        });

        Self {
            vm,
            prover_config,
            prover_state: Mutex::new(HashMap::new()),
            zk_storage,
        }
    }
}

#[async_trait]
impl<StateRoot, Witness, Da, Vm, V> ProverService for ParallelProver<StateRoot, Witness, Da, Vm, V>
where
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync,
    Witness: Serialize + DeserializeOwned + Send + Sync,
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
        let header_hash = state_transition_data.da_block_header.hash().into();
        let data = ProverState::WitnessSubmitted(state_transition_data);
        self.prover_state
            .lock()
            .expect("Lock was poisoned")
            .insert(header_hash, data);
    }

    async fn prove(&self, block_header_hash: Hash) -> Result<(), ProverServiceError> {
        if let Some(config) = self.prover_config.clone() {
            let prover_state = self
                .prover_state
                .lock()
                .expect("Lock was poisoned")
                .remove(&block_header_hash)
                .unwrap(); // TODO

            match prover_state {
                ProverState::WitnessSubmitted(transition_data) => {
                    let config = config.clone();

                    let mut vm = self.vm.clone();
                    vm.add_hint(transition_data);

                    let zk_storage = self.zk_storage.clone();

                    start_proving(config, vm, zk_storage);
                }
                ProverState::Err(e) => return Err(e.into()),
                _ => return Err(anyhow::anyhow!("Proof was already send to the prover").into()),
            }
        }
        Ok(())
    }
}

fn start_proving<Da, Vm, V>(
    config: Arc<ProofGenConfig<V, Da, Vm>>,
    mut vm: Vm,
    zk_storage: V::PreState,
) where
    Da: DaService,
    Vm: ZkvmHost + 'static,
    V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync + 'static,
    V::PreState: Send + Sync,
{
    rayon::spawn(move || {
        tracing::info_span!("guest_execution").in_scope(|| {
            let _proof = match config.deref() {
                ProofGenConfig::Simulate(verifier) => verifier
                    .run_block(vm.simulate_with_hints(), zk_storage)
                    .map_err(|e| {
                        anyhow::anyhow!("Guest execution must succeed but failed with {:?}", e)
                    })
                    .map(|_| ()),
                ProofGenConfig::Execute => {
                    let _ = vm.run(false).unwrap();
                    Ok(())
                }
                ProofGenConfig::Prover => {
                    let _ = vm.run(true).unwrap();
                    Ok(())
                }
            };
        })
    });
}
