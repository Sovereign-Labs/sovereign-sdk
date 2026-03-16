use std::marker::PhantomData;

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
    ZkvmHost, ZkvmNetwork,
};
use tracing::{error, info, trace};

use super::state::{NetworkProverState, NetworkProverStatus, SubmittedProofMetadata};
use super::Verifier;
use crate::processes::prover_service::block_proof::BlockProof;
use crate::processes::{
    ProofAggregationStatus, ProofProcessingStatus, ProverServiceError, StateTransitionInfo,
};

pub(crate) struct NetworkProver<Address, StateRoot, Witness, Da: DaService, InnerVm: Zkvm> {
    prover_address: Address,
    inner_network: tokio::sync::Mutex<InnerVm::Network>,
    prover_state: tokio::sync::RwLock<
        NetworkProverState<
            Address,
            StateRoot,
            Da::Spec,
            <InnerVm::Network as ZkvmNetwork>::ProofHandle,
        >,
    >,
    code_commitment: CodeCommitment,
    phantom: PhantomData<Witness>,
}

impl<Address, StateRoot, Witness, Da, InnerVm>
    NetworkProver<Address, StateRoot, Witness, Da, InnerVm>
where
    Da: DaService,
    Address: BorshSerialize
        + Serialize
        + DeserializeOwned
        + AsRef<[u8]>
        + Clone
        + Send
        + Sync
        + 'static,
    StateRoot: Serialize + DeserializeOwned + Clone + AsRef<[u8]> + Send + Sync + 'static,
    Witness: Serialize + DeserializeOwned + Send + Sync + 'static,
    InnerVm: Zkvm + 'static,
{
    pub(crate) fn new(
        prover_address: Address,
        inner_network: InnerVm::Network,
        code_commitment: CodeCommitment,
    ) -> Self {
        Self {
            prover_address,
            inner_network: tokio::sync::Mutex::new(inner_network),
            prover_state: tokio::sync::RwLock::new(NetworkProverState {
                prover_status: Default::default(),
            }),
            code_commitment,
            phantom: PhantomData,
        }
    }

    pub(crate) async fn start_proving(
        &self,
        state_transition_info: StateTransitionInfo<StateRoot, Witness, Da::Spec>,
        verifier: &Verifier<Da>,
    ) -> Result<ProofProcessingStatus<StateRoot, Witness, Da::Spec>, ProverServiceError> {
        let block_header_hash = state_transition_info.da_block_header().hash();

        let mut prover_state = self.prover_state.write().await;

        if let Some(status) = prover_state.get_prover_status(&block_header_hash) {
            return match status {
                NetworkProverStatus::Submitted { .. } => Err(anyhow::anyhow!(
                    "Proof generation for {} still in progress",
                    block_header_hash,
                )
                .into()),
                NetworkProverStatus::Proved(_) => Err(anyhow::anyhow!(
                    "Witness for block_header_hash {}, submitted multiple times.",
                    block_header_hash,
                )
                .into()),
                NetworkProverStatus::Err(e) => Err(anyhow::format_err!("{}", e).into()),
            };
        }

        let slot_number = state_transition_info.slot_number;

        let data = StateTransitionWitnessWithAddress {
            stf_witness: state_transition_info.data,
            prover_address: self.prover_address.clone(),
        };

        // Add hint to the network prover (borrows data via serialization).
        let mut network = self.inner_network.lock().await;
        network.add_hint(&data);

        // Destructure the witness data so we can verify DA inclusion proofs
        // before submitting to the network, failing fast before spending credits.
        let StateTransitionWitnessWithAddress {
            stf_witness:
                StateTransitionWitness {
                    initial_state_root,
                    final_state_root,
                    da_block_header,
                    relevant_proofs,
                    relevant_blobs,
                    ..
                },
            prover_address,
        } = data;

        verifier
            .da_verifier
            .verify_relevant_tx_list(&da_block_header, &relevant_blobs, relevant_proofs)
            .map_err(|e| {
                ProverServiceError::Other(anyhow::anyhow!("DA verification failed: {:?}", e))
            })?;

        let metadata = SubmittedProofMetadata {
            slot_number,
            st: StateTransitionPublicData {
                initial_state_root,
                final_state_root,
                slot_hash: block_header_hash.clone(),
                prover_address,
            },
        };

        info!(
            "Submitting proof to network for slot hash {}",
            block_header_hash
        );
        let handle = match network.submit().await {
            Ok(handle) => handle,
            Err(e) => {
                return Err(ProverServiceError::Other(anyhow::anyhow!(
                    "Failed to submit proof to network: {}",
                    e
                )));
            }
        };

        prover_state.set_to_submitted(block_header_hash, handle, metadata);

        Ok(ProofProcessingStatus::ProvingInProgress)
    }

    pub(crate) async fn create_aggregated_proof<OuterVm: ZkvmHost + 'static>(
        &self,
        mut outer_vm: OuterVm,
        block_header_hashes: &[<Da::Spec as DaSpec>::SlotHash],
        genesis_state_root: &StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus> {
        assert!(!block_header_hashes.is_empty());

        let mut prover_state = self.prover_state.write().await;

        // Phase 1: Poll all submitted entries and transition them to Proved.
        // We remove-then-reinsert to avoid holding immutable references across mutations.
        for slot_hash in block_header_hashes {
            if let Some(NetworkProverStatus::Submitted { handle, metadata }) =
                prover_state.remove(slot_hash)
            {
                let network = self.inner_network.lock().await;
                match network.poll(&handle).await {
                    Ok(Some(proof_bytes)) => {
                        let block_proof = BlockProof {
                            _proof: proof_bytes,
                            slot_number: metadata.slot_number,
                            st: metadata.st,
                        };
                        prover_state.set_to_proved(slot_hash.clone(), block_proof);
                    }
                    Ok(None) => {
                        info!(
                            "Proof for slot hash {} is still pending on the network",
                            slot_hash
                        );
                        // Put back as Submitted since it's not done yet
                        prover_state.set_to_submitted(slot_hash.clone(), handle, metadata);
                        return Ok(ProofAggregationStatus::ProofGenerationInProgress);
                    }
                    Err(e) => {
                        error!("Network proof for slot hash {} failed: {:?}", slot_hash, e);
                        prover_state.set_to_err(
                            slot_hash.clone(),
                            anyhow::anyhow!("Network proving failed: {}", e),
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

        // Phase 2: Collect all proved block proofs.
        let mut block_proofs_data = Vec::new();
        for slot_hash in block_header_hashes {
            match prover_state.get_prover_status(slot_hash) {
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

        trace!(%public_data, "generating aggregate proof");
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
