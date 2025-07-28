use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use borsh::BorshSerialize;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec, DaVerifier};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, CodeCommitment, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{
    StateTransitionPublicData, StateTransitionWitness, StateTransitionWitnessWithAddress, Zkvm,
    ZkvmHost,
};
use tracing::{error, info, trace};

use super::state::{ProverState, ProverStatus};
use super::{ProverServiceError, Verifier};
use crate::processes::prover_service::block_proof::BlockProof;
use crate::processes::{
    ProofAggregationStatus, ProofProcessingStatus, RollupProverConfigDiscriminants,
    StateTransitionInfo,
};

// A prover that generates proofs in parallel using a thread pool. If the pool is saturated,
// the prover will reject new jobs.
pub(crate) struct Prover<Address, StateRoot, Witness, Da: DaService> {
    prover_address: Address,
    prover_state: Arc<RwLock<ProverState<Address, StateRoot, Da::Spec>>>,
    num_threads: usize,
    // From Docs:
    // """
    // When the ThreadPool is dropped,
    // that's a signal for the threads it manages to terminate,
    // they will complete executing any remaining work that you have spawned,
    // and automatically terminate.
    // """
    pool: rayon::ThreadPool,
    code_commitment: CodeCommitment,
    phantom: std::marker::PhantomData<(StateRoot, Witness, Da)>,
}

impl<Address, StateRoot, Witness, Da> Prover<Address, StateRoot, Witness, Da>
where
    Da: DaService,
    Address:
        BorshSerialize + Serialize + DeserializeOwned + AsRef<[u8]> + Clone + Send + Sync + 'static,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    pub(crate) fn new(
        prover_address: Address,
        num_threads: usize,
        code_commitment: CodeCommitment,
    ) -> Self {
        Self {
            code_commitment,
            num_threads,
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(num_threads)
                .build()
                .unwrap(),

            prover_state: Arc::new(RwLock::new(ProverState {
                prover_status: Default::default(),
                pending_tasks_count: Default::default(),
            })),
            prover_address,
            phantom: PhantomData,
        }
    }

    pub(crate) fn start_proving<InnerVm>(
        &self,
        state_transition_info: StateTransitionInfo<StateRoot, Witness, <Da as DaService>::Spec>,
        config: RollupProverConfigDiscriminants,
        mut inner_vm: InnerVm::Host,
        verifier: Arc<Verifier<Da>>,
    ) -> Result<
        ProofProcessingStatus<StateRoot, Witness, <Da as DaService>::Spec>,
        ProverServiceError,
    >
    where
        InnerVm: Zkvm + 'static,
    {
        let block_header_hash = state_transition_info.da_block_header().hash();

        let mut prover_state = self.prover_state.write().expect("Lock was poisoned");
        if let Some(duplicate_proof) = prover_state.get_prover_status(&block_header_hash) {
            return match duplicate_proof {
                ProverStatus::ProvingInProgress => Err(anyhow::anyhow!(
                    "Proof generation for {} still in progress",
                    block_header_hash
                )
                .into()),
                ProverStatus::Proved(_) => Err(anyhow::anyhow!(
                    "Witness for block_header_hash {}, submitted multiple times.",
                    block_header_hash,
                )
                .into()),
                ProverStatus::Err(e) => Err(anyhow::format_err!("{}", e).into()), // "Clone" the anyhow error without cloning, because anyhow doesn't support that
            };
        }

        let start_prover = prover_state.inc_task_count_if_not_busy(self.num_threads);

        let prover_state_clone = self.prover_state.clone();
        // Initiate a new proving job only if the prover is not busy.
        if start_prover {
            prover_state.set_to_proving(block_header_hash.clone());

            let data = StateTransitionWitnessWithAddress {
                stf_witness: state_transition_info.data,
                prover_address: self.prover_address.clone(),
            };

            let prover_address = self.prover_address.clone();

            inner_vm.add_hint(&data);

            self.pool.spawn(move || {
                tracing::info_span!("guest_execution").in_scope(|| {
                    let proof = make_inner_proof::<InnerVm>(inner_vm, config);

                    let mut prover_state = prover_state_clone.write().expect("Lock was poisoned");

                    let StateTransitionWitness {
                        initial_state_root,
                        final_state_root,
                        da_block_header,
                        relevant_proofs,
                        relevant_blobs: blobs,
                        ..
                    } = data.stf_witness;

                    verifier
                        .da_verifier
                        .verify_relevant_tx_list(&da_block_header, &blobs, relevant_proofs)
                        .expect("An honest prover provided an invalid list of relevant txs. This is a bug in the prover - please report it.");

                    let block_proof = proof.map(|p| BlockProof {
                        _proof: p,
                        st: StateTransitionPublicData::<Address, Da::Spec, StateRoot> {
                            initial_state_root,
                            final_state_root,
                            slot_hash: block_header_hash.clone(),
                            prover_address,
                        },
                        slot_number: state_transition_info.slot_number,
                    });

                    prover_state.set_to_proved(block_header_hash, block_proof);
                    prover_state.dec_task_count();
                });
            });

            Ok(ProofProcessingStatus::ProvingInProgress)
        } else {
            Ok(ProofProcessingStatus::Busy(state_transition_info))
        }
    }

    pub(crate) fn create_aggregated_proof<OuterVm: ZkvmHost + 'static>(
        &self,
        mut outer_vm: OuterVm,
        block_header_hashes: &[<Da::Spec as DaSpec>::SlotHash],
        genesis_state_root: &StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus> {
        assert!(!block_header_hashes.is_empty());
        let mut prover_state = self.prover_state.write().expect("Lock was poisoned");

        let mut block_proofs_data = Vec::default();

        for slot_hash in block_header_hashes {
            let state = prover_state.get_prover_status(slot_hash);

            match state {
                Some(ProverStatus::ProvingInProgress) => {
                    return Ok(ProofAggregationStatus::ProofGenerationInProgress);
                }
                Some(ProverStatus::Proved(block_proof)) => {
                    assert_eq!(slot_hash, &block_proof.st.slot_hash);
                    block_proofs_data.push(block_proof);
                }
                Some(ProverStatus::Err(e)) => return Err(anyhow::anyhow!(e.to_string())),
                None => return Err(anyhow::anyhow!("Missing required proof of {:?}. Use the `prove` method to generate a proof of that block and try again.", slot_hash)),
            }
        }

        // It is ok to unwrap here as we asserted that block_proofs_data.len() >= 1.
        let initial_block_proof = block_proofs_data.first().unwrap();
        let final_block_proof = block_proofs_data.last().unwrap();

        let mut rewarded_addresses = Vec::default();
        for bp in block_proofs_data.iter() {
            rewarded_addresses.push(bp.st.prover_address.clone());
        }

        let public_data = AggregatedProofPublicData::<Address, Da::Spec, StateRoot> {
            rewarded_addresses,
            initial_slot_number: initial_block_proof.slot_number,
            final_slot_number: final_block_proof.slot_number,
            genesis_state_root: genesis_state_root.clone(),
            initial_state_root: initial_block_proof.st.initial_state_root.clone(),
            final_state_root: final_block_proof.st.final_state_root.clone(),
            initial_slot_hash: initial_block_proof.st.slot_hash.clone(),
            final_slot_hash: final_block_proof.st.slot_hash.clone(),
            code_commitment: self.code_commitment.clone(),
        };

        trace!(%public_data, "generating aggregate proof");
        // TODO: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/316
        // `add_hint`  should take witness instead of the public input.
        outer_vm.add_hint(public_data);
        let serialized_aggregated_proof = SerializedAggregatedProof {
            raw_aggregated_proof: outer_vm.run(false)?,
        };

        for slot_hash in block_header_hashes {
            prover_state.remove(slot_hash);
        }
        Ok(ProofAggregationStatus::Success(serialized_aggregated_proof))
    }
}

fn make_inner_proof<InnerVm>(
    mut vm: InnerVm::Host,
    config: RollupProverConfigDiscriminants,
) -> anyhow::Result<Vec<u8>>
where
    InnerVm: Zkvm + 'static,
{
    let proving_start = std::time::Instant::now();
    let result = match config {
        RollupProverConfigDiscriminants::Skip => Ok(Vec::default()),
        RollupProverConfigDiscriminants::Execute => {
            info!(
                "Executing in VM without constructing proof using {}",
                std::any::type_name::<InnerVm>()
            );
            vm.run(false)
        }
        RollupProverConfigDiscriminants::Prove => {
            info!("Generating proof with {}", std::any::type_name::<InnerVm>());
            vm.run(true)
        }
    };
    sov_metrics::track_metrics(|tracker| {
        let proving_time = proving_start.elapsed();
        let is_success = result.is_ok();
        tracker.submit(sov_metrics::ZkProvingTime {
            proving_time,
            is_success,
            zk_circuit: sov_metrics::ZkCircuit::Inner,
        });
    });
    match result {
        Ok(ref proof) => {
            trace!(
                bytes = proof.len(),
                "Proof generation completed successfully"
            );
        }
        Err(ref e) => {
            error!("Proof generation failed: {:?}", e);
        }
    }
    result
}
