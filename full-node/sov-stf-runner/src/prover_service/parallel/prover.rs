use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use super::{Hash, ProverServiceError};

use crate::{ProofGenConfig, ProofSubmissionStatus, StateTransitionData};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::{Proof, ZkvmHost};

struct ZkProofProducer {}

impl ZkProofProducer {
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
            ProofGenConfig::Simulate(verifier) => verifier
                .run_block(vm.simulate_with_hints(), zk_storage)
                .map(|_| Proof::Empty)
                .map_err(|e| {
                    anyhow::anyhow!("Guest execution must succeed but failed with {:?}", e)
                }),
            ProofGenConfig::Execute => vm.run(false),
            ProofGenConfig::Prover => vm.run(true),
        }
    }
}

enum ProverStatus<StateRoot, Witness, Da: DaSpec> {
    WitnessSubmitted(StateTransitionData<StateRoot, Witness, Da>),
    Proving,
    Proved(Proof),
    Err(anyhow::Error),
}

#[derive(Default)]
struct ProverState<StateRoot, Witness, Da: DaSpec> {
    prover_status: HashMap<Hash, ProverStatus<StateRoot, Witness, Da>>,
}

impl<StateRoot, Witness, Da: DaSpec> ProverState<StateRoot, Witness, Da> {
    fn remove(&mut self, hash: &Hash) -> Option<ProverStatus<StateRoot, Witness, Da>> {
        self.prover_status.remove(hash)
    }

    fn set_to_proving(&mut self, hash: Hash) -> Option<ProverStatus<StateRoot, Witness, Da>> {
        self.prover_status.insert(hash, ProverStatus::Proving)
    }

    fn set_to_proved(
        &mut self,
        hash: Hash,
        proof: Result<Proof, anyhow::Error>,
    ) -> Option<ProverStatus<StateRoot, Witness, Da>> {
        match proof {
            Ok(p) => self.prover_status.insert(hash, ProverStatus::Proved(p)),
            Err(e) => self.prover_status.insert(hash, ProverStatus::Err(e)),
        }
    }

    fn get_prover_status(&self, hash: Hash) -> Option<&ProverStatus<StateRoot, Witness, Da>> {
        self.prover_status.get(&hash)
    }
}

pub(crate) struct Prover<StateRoot, Witness, Da: DaService> {
    prover_state: Arc<Mutex<ProverState<StateRoot, Witness, Da::Spec>>>,
}

impl<StateRoot, Witness, Da> Prover<StateRoot, Witness, Da>
where
    Da: DaService,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    pub(crate) fn new() -> Self {
        Self {
            prover_state: Arc::new(Mutex::new(ProverState {
                prover_status: Default::default(),
            })),
        }
    }

    pub(crate) fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<StateRoot, Witness, Da::Spec>,
    ) {
        let header_hash = state_transition_data.da_block_header.hash().into();
        let data = ProverStatus::WitnessSubmitted(state_transition_data);
        self.prover_state
            .lock()
            .unwrap()
            .prover_status
            .insert(header_hash, data);
    }

    pub(crate) fn start_proving<Vm, V>(
        &self,
        block_header_hash: Hash,
        config: Arc<ProofGenConfig<V, Da, Vm>>,
        mut vm: Vm,
        zk_storage: V::PreState,
    ) -> Result<(), ProverServiceError>
    where
        Vm: ZkvmHost + 'static,
        V: StateTransitionFunction<Vm::Guest, Da::Spec> + Send + Sync + 'static,
        V::PreState: Send + Sync + 'static,
    {
        let prover_state_clone = self.prover_state.clone();

        let mut prover_state = self.prover_state.lock().expect("Lock was poisoned");
        let prover_status = prover_state.remove(&block_header_hash).unwrap(); // TODO

        match prover_status {
            ProverStatus::WitnessSubmitted(state_tranistion_data) => {
                prover_state.set_to_proving(block_header_hash);
                vm.add_hint(state_tranistion_data);

                rayon::spawn(move || {
                    tracing::info_span!("guest_execution").in_scope(|| {
                        let proof = ZkProofProducer::make_proof(vm, config, zk_storage);
                        prover_state_clone
                            .lock()
                            .expect("Lock was poisoned")
                            .set_to_proved(block_header_hash, proof);
                    })
                });

                Ok(())
            }
            ProverStatus::Proving => todo!(),
            ProverStatus::Proved(_) => todo!(),
            ProverStatus::Err(e) => Err(e.into()),
        }
    }

    pub(crate) fn get_proof_submission_status(
        &self,
        block_header_hash: Hash,
    ) -> ProofSubmissionStatus {
        let prover_state = self.prover_state.lock().unwrap();
        let status = prover_state.get_prover_status(block_header_hash);

        match status {
            Some(ProverStatus::Proving) => ProofSubmissionStatus::ProvingInProgress,
            Some(ProverStatus::Proved(_)) => ProofSubmissionStatus::Success,
            Some(ProverStatus::WitnessSubmitted(_)) => {
                ProofSubmissionStatus::Err(anyhow::anyhow!(""))
            }
            Some(ProverStatus::Err(e)) => {
                ProofSubmissionStatus::Err(anyhow::anyhow!(e.to_string()))
            }
            None => ProofSubmissionStatus::Err(anyhow::anyhow!("")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sov_mock_da::{MockDaConfig, MockDaService, MockDaSpec};
    use sov_modules_api::Zkvm;

    struct C {}

    struct TestStf {}

    impl StateTransitionFunction<TestVm, MockDaSpec> for TestStf {
        type StateRoot = Vec<u8>;

        type GenesisParams = ();

        type PreState = ();

        type ChangeSet = ();

        type TxReceiptContents = ();

        type BatchReceiptContents = ();

        type Witness = ();

        type Condition = ();

        fn init_chain(
            &self,
            genesis_state: Self::PreState,
            params: Self::GenesisParams,
        ) -> (Self::StateRoot, Self::ChangeSet) {
            todo!()
        }

        fn apply_slot<'a, I>(
            &self,
            pre_state_root: &Self::StateRoot,
            pre_state: Self::PreState,
            witness: Self::Witness,
            slot_header: &<MockDaSpec as DaSpec>::BlockHeader,
            validity_condition: &<MockDaSpec as DaSpec>::ValidityCondition,
            blobs: I,
        ) -> sov_rollup_interface::stf::SlotResult<
            Self::StateRoot,
            Self::ChangeSet,
            Self::BatchReceiptContents,
            Self::TxReceiptContents,
            Self::Witness,
        >
        where
            I: IntoIterator<Item = &'a mut <MockDaSpec as DaSpec>::BlobTransaction>,
        {
            todo!()
        }
    }

    #[derive(Clone)]
    struct TestVm {}

    impl Zkvm for TestVm {
        type CodeCommitment = Vec<u8>;

        type Error = ();

        fn verify<'a>(
            serialized_proof: &'a [u8],
            code_commitment: &Self::CodeCommitment,
        ) -> Result<&'a [u8], Self::Error> {
            todo!()
        }

        fn verify_and_extract_output<
            Add: sov_rollup_interface::RollupAddress,
            Da: DaSpec,
            Root: Serialize + DeserializeOwned,
        >(
            serialized_proof: &[u8],
            code_commitment: &Self::CodeCommitment,
        ) -> Result<sov_modules_api::StateTransition<Da, Add, Root>, Self::Error> {
            todo!()
        }
    }

    impl ZkvmHost for TestVm {
        type Guest;

        fn add_hint<T: Serialize>(&mut self, item: T) {
            todo!()
        }

        fn simulate_with_hints(&mut self) -> Self::Guest {
            todo!()
        }

        fn run(&mut self, with_proof: bool) -> Result<Proof, anyhow::Error> {
            todo!()
        }
    }

    #[tokio::test]
    async fn test_something_async() {
        let prover = Prover::<Vec<u8>, Vec<u8>, MockDaService>::new();

        let block_header_hash = [0; 32];
        let test_vm = TestVm {};
        let config = Arc::new(ProofGenConfig::Execute);

        prover.start_proving(block_header_hash, config, test_vm, zk_storage);
    }
}
