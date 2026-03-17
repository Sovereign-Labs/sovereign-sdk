use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use borsh::BorshSerialize;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec, DaVerifier};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, CodeCommitment, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{
    StateTransitionPublicData, StateTransitionWitness, StateTransitionWitnessWithAddress, Zkvm,
    ZkvmNetwork,
};

use super::Verifier;
use crate::processes::prover_service::block_proof::BlockProof;
use crate::processes::{
    ProofAggregationStatus, ProofProcessingStatus, ProverServiceError, StateTransitionInfo,
};

struct SubmittedProofMetadata<Address, Da: DaSpec, StateRoot> {
    pub(crate) slot_number: SlotNumber,
    pub(crate) st: StateTransitionPublicData<Address, Da, StateRoot>,
}

enum NetworkProverStatus<Address, StateRoot, Da: DaSpec, Handle> {
    Submitted {
        handle: Handle,
        metadata: SubmittedProofMetadata<Address, Da, StateRoot>,
    },
    Proved(BlockProof<Address, Da, StateRoot>),
    Err(anyhow::Error),
}

type ProofStatusMap<Address, StateRoot, Da, InnerVm> = HashMap<
    <Da as DaSpec>::SlotHash,
    NetworkProverStatus<
        Address,
        StateRoot,
        Da,
        <<InnerVm as Zkvm>::Network as ZkvmNetwork>::ProofHandle,
    >,
>;

pub(crate) struct NetworkProver<
    Address,
    StateRoot,
    Witness,
    Da: DaService,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
> {
    prover_address: Address,
    inner_vm: tokio::sync::Mutex<InnerVm::Network>,
    outer_vm: Arc<tokio::sync::Mutex<OuterVm::Network>>,
    tracker: tokio::sync::RwLock<ProofStatusMap<Address, StateRoot, Da::Spec, InnerVm>>,
    code_commitment: CodeCommitment,
    aggregation_task: tokio::sync::Mutex<Option<tokio::task::AbortHandle>>,
    phantom: PhantomData<Witness>,
}

impl<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
    NetworkProver<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
where
    Da: DaService,
    Address:
        BorshSerialize + Serialize + DeserializeOwned + AsRef<[u8]> + Clone + Send + Sync + 'static,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
    InnerVm: Zkvm + 'static,
    OuterVm: Zkvm + 'static,
{
    pub(crate) fn new(
        prover_address: Address,
        inner_vm: InnerVm::Network,
        outer_vm: OuterVm::Network,
        code_commitment: CodeCommitment,
    ) -> Self {
        Self {
            prover_address,
            inner_vm: tokio::sync::Mutex::new(inner_vm),
            outer_vm: Arc::new(tokio::sync::Mutex::new(outer_vm)),
            tracker: tokio::sync::RwLock::new(HashMap::new()),
            code_commitment,
            aggregation_task: tokio::sync::Mutex::new(None),
            phantom: PhantomData,
        }
    }

    pub(crate) fn proving_precondition(
        &self,
        state_transition_info: &StateTransitionInfo<StateRoot, Witness, Da::Spec>,
    ) -> anyhow::Result<()> {
        let block_header_hash = state_transition_info.da_block_header().hash();
        let tracker = self.tracker.blocking_read();

        if let Some(status) = tracker.get(&block_header_hash) {
            return match status {
                NetworkProverStatus::Submitted { .. } => Err(anyhow::anyhow!(
                    "Proof generation for {} still in progress",
                    block_header_hash,
                )),
                NetworkProverStatus::Proved(_) => Err(anyhow::anyhow!(
                    "Witness for block_header_hash {}, submitted multiple times.",
                    block_header_hash,
                )),
                NetworkProverStatus::Err(e) => Err(anyhow::format_err!("{}", e)),
            };
        }

        Ok(())
    }

    pub(crate) async fn start_proving(
        &self,
        state_transition_info: StateTransitionInfo<StateRoot, Witness, Da::Spec>,
        verifier: &Verifier<Da>,
    ) -> Result<ProofProcessingStatus<StateRoot, Witness, Da::Spec>, ProverServiceError> {
        self.proving_precondition(&state_transition_info)?;

        let block_header_hash = state_transition_info.da_block_header().hash();
        let slot_number = state_transition_info.slot_number;
        let data = StateTransitionWitnessWithAddress {
            stf_witness: state_transition_info.data,
            prover_address: self.prover_address.clone(),
        };

        verifier
            .da_verifier
            .verify_relevant_tx_list(
                &data.stf_witness.da_block_header,
                &data.stf_witness.relevant_blobs,
                &data.stf_witness.relevant_proofs,
            )
            .map_err(|e| {
                ProverServiceError::Other(anyhow::anyhow!("DA verification failed: {:?}", e))
            })?;

        let handle = {
            let mut network = self.inner_vm.lock().await;
            network.add_hint(&data);
            // TODO: what happens if we crash here? Do we just pay to re-prove?
            network.submit().await.map_err(ProverServiceError::Other)?
        };

        let StateTransitionWitnessWithAddress {
            stf_witness:
                StateTransitionWitness {
                    initial_state_root,
                    final_state_root,
                    ..
                },
            prover_address,
        } = data;

        let metadata = SubmittedProofMetadata {
            slot_number,
            st: StateTransitionPublicData {
                initial_state_root,
                final_state_root,
                slot_hash: block_header_hash.clone(),
                prover_address,
            },
        };

        tracing::trace!(
            "Submitting proof to network for slot hash {}",
            block_header_hash
        );

        let mut tracker = self.tracker.write().await;
        tracker.insert(
            block_header_hash,
            NetworkProverStatus::Submitted { handle, metadata },
        );

        Ok(ProofProcessingStatus::ProvingInProgress)
    }

    pub(crate) async fn create_aggregated_proof(
        &self,
        block_header_hashes: &[<Da::Spec as DaSpec>::SlotHash],
        genesis_state_root: &StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus> {
        assert!(!block_header_hashes.is_empty());

        let mut proof_statuses = self.tracker.write().await;
        // Phase 1: Poll all submitted entries and transition them to Proved.
        // We remove-then-reinsert to avoid holding immutable references across mutations.
        for slot_hash in block_header_hashes {
            if let Some(NetworkProverStatus::Submitted { handle, metadata }) =
                proof_statuses.remove(slot_hash)
            {
                let network = self.inner_vm.lock().await;
                match network.poll(&handle).await {
                    Ok(Some(proof_bytes)) => {
                        let block_proof = BlockProof {
                            _proof: proof_bytes,
                            slot_number: metadata.slot_number,
                            st: metadata.st,
                        };
                        proof_statuses
                            .insert(slot_hash.clone(), NetworkProverStatus::Proved(block_proof));
                    }
                    Ok(None) => {
                        tracing::trace!(
                            "Proof for slot hash {} is still pending on the network",
                            slot_hash
                        );
                        // Put back as Submitted since it's not done yet
                        proof_statuses.insert(
                            slot_hash.clone(),
                            NetworkProverStatus::Submitted { handle, metadata },
                        );
                        return Ok(ProofAggregationStatus::ProofGenerationInProgress);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Network proof for slot hash {} failed: {:?}",
                            slot_hash,
                            e
                        );
                        proof_statuses.insert(
                            slot_hash.clone(),
                            NetworkProverStatus::Err(anyhow::anyhow!(
                                "Network proving failed: {}",
                                e
                            )),
                        );
                        return Err(anyhow::anyhow!(
                            "Network proving failed for {}: {}",
                            slot_hash,
                            e
                        ));
                    }
                }
            }
        }

        let proof_statuses = proof_statuses.downgrade();
        // Phase 2: Collect all proved block proofs.
        let mut block_proofs_data = Vec::new();
        for slot_hash in block_header_hashes {
            match proof_statuses.get(slot_hash) {
                Some(NetworkProverStatus::Proved(block_proof)) => {
                    assert_eq!(slot_hash, &block_proof.st.slot_hash);
                    block_proofs_data.push(block_proof);
                }
                Some(NetworkProverStatus::Err(e)) => {
                    return Err(anyhow::anyhow!(e.to_string()));
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "Missing required proof of {:?}. Use the `prove` method to generate a proof of that block and try again.",
                        slot_hash
                    ));
                }
                Some(NetworkProverStatus::Submitted { .. }) => {
                    unreachable!("All Submitted entries should have been resolved in phase 1")
                }
            }
        }

        let initial_block_proof = block_proofs_data.first().unwrap();
        let final_block_proof = block_proofs_data.last().unwrap();

        let mut rewarded_addresses = Vec::new();
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

        tracing::trace!(%public_data, "generating aggregate proof");

        // Drop the read lock before submitting to the outer network.
        drop(proof_statuses);

        let handle = {
            let mut outer = self.outer_vm.lock().await;
            outer.add_hint(&public_data);
            outer.submit().await?
        };

        let outer_vm = Arc::clone(&self.outer_vm);
        let task = tokio::spawn(async move {
            loop {
                let outer = outer_vm.lock().await;
                match outer.poll(&handle).await {
                    Ok(Some(proof_bytes)) => {
                        return Ok(SerializedAggregatedProof {
                            raw_aggregated_proof: proof_bytes,
                        });
                    }
                    Ok(None) => {
                        drop(outer);
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!("Outer network proving failed: {}", e));
                    }
                }
            }
        });

        *self.aggregation_task.lock().await = Some(task.abort_handle());

        let result = task
            .await
            .map_err(|e| anyhow::anyhow!("Aggregation task panicked: {}", e))?;

        *self.aggregation_task.lock().await = None;

        let serialized_aggregated_proof = result?;

        let mut tracker = self.tracker.write().await;
        for slot_hash in block_header_hashes {
            tracker.remove(slot_hash);
        }

        Ok(ProofAggregationStatus::Success(serialized_aggregated_proof))
    }
}

impl<Address, StateRoot, Witness, Da, InnerVm, OuterVm> Drop
    for NetworkProver<Address, StateRoot, Witness, Da, InnerVm, OuterVm>
where
    Da: DaService,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
{
    fn drop(&mut self) {
        if let Some(task) = self.aggregation_task.get_mut().take() {
            task.abort();
        }
    }
}
