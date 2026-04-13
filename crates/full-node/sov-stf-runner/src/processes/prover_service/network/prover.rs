use borsh::BorshSerialize;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec, DaVerifier};
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, BlockProof, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::{
    StateTransitionPublicData, StateTransitionWitness, StateTransitionWitnessWithAddress, Zkvm,
    ZkvmNetwork,
};
use std::collections::HashMap;
use std::marker::PhantomData;

use super::Verifier;
use crate::processes::{
    ProofAggregationStatus, ProofProcessingStatus, ProverServiceError, StateTransitionInfo,
};

struct SubmittedProofMetadata<Address, Da: DaSpec, StateRoot> {
    slot_number: SlotNumber,
    st: StateTransitionPublicData<Address, Da, StateRoot>,
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
    inner_vm: InnerVm::Network,
    outer_vm: OuterVm::Network,
    tracker: tokio::sync::RwLock<ProofStatusMap<Address, StateRoot, Da::Spec, InnerVm>>,
    outer_proof_timeout: std::time::Duration,
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
        outer_proof_timeout: std::time::Duration,
    ) -> Self {
        Self {
            prover_address,
            inner_vm,
            outer_vm,
            tracker: tokio::sync::RwLock::new(HashMap::new()),
            outer_proof_timeout,
            phantom: PhantomData,
        }
    }

    pub(crate) async fn start_proving(
        &self,
        state_transition_info: StateTransitionInfo<StateRoot, Witness, Da::Spec>,
        verifier: &Verifier<Da>,
    ) -> Result<ProofProcessingStatus<StateRoot, Witness, Da::Spec>, ProverServiceError> {
        let block_header_hash = state_transition_info.da_block_header().hash();

        {
            let tracker = self.tracker.read().await;
            if let Some(status) = tracker.get(&block_header_hash) {
                return match status {
                    NetworkProverStatus::Submitted { .. } => {
                        Err(ProverServiceError::Other(anyhow::anyhow!(
                            "Proof generation for {} still in progress",
                            block_header_hash,
                        )))
                    }
                    NetworkProverStatus::Proved(_) => {
                        Err(ProverServiceError::Other(anyhow::anyhow!(
                            "Witness for block_header_hash {}, submitted multiple times.",
                            block_header_hash,
                        )))
                    }
                    NetworkProverStatus::Err(e) => {
                        Err(ProverServiceError::Other(anyhow::format_err!("{}", e)))
                    }
                };
            }
        }

        let slot_number = state_transition_info.slot_number;
        let data = StateTransitionWitnessWithAddress {
            stf_witness: state_transition_info.data,
            prover_address: self.prover_address.clone(),
        };

        // Verify DA BEFORE submitting to the network to avoid leaking a proof handle
        verifier
            .da_verifier
            .verify_relevant_tx_list(
                &data.stf_witness.da_block_header,
                &data.stf_witness.relevant_blobs,
                data.stf_witness.relevant_proofs.clone(),
            )
            .map_err(|e| {
                ProverServiceError::Other(anyhow::anyhow!("DA verification failed: {:?}", e))
            })?;

        let handle = self
            .inner_vm
            .add_hint_and_submit(&data)
            .await
            .map_err(ProverServiceError::Other)?;

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

        {
            let mut tracker = self.tracker.write().await;
            tracker.insert(
                block_header_hash,
                NetworkProverStatus::Submitted { handle, metadata },
            );
        }

        Ok(ProofProcessingStatus::ProvingInProgress)
    }

    pub(crate) async fn create_aggregated_proof(
        &self,
        block_header_hashes: &[<Da::Spec as DaSpec>::SlotHash],
        genesis_state_root: &StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus> {
        assert!(!block_header_hashes.is_empty());

        let mut proof_statuses = self.tracker.write().await;
        // Phase 1: Poll all Submitted entries and transition them to Proved.
        // We only remove entries confirmed to be Submitted, so Proved/Err entries are untouched.
        for slot_hash in block_header_hashes {
            // We want to only remove if we know the entry exists
            if !matches!(
                proof_statuses.get(slot_hash),
                Some(NetworkProverStatus::Submitted { .. })
            ) {
                continue;
            }

            let Some(NetworkProverStatus::Submitted { handle, metadata }) =
                proof_statuses.remove(slot_hash)
            else {
                unreachable!()
            };

            match self.inner_vm.poll(&handle).await {
                Ok(Some(proof_bytes)) => {
                    let block_proof = BlockProof {
                        proof: proof_bytes,
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
                    proof_statuses.insert(
                        slot_hash.clone(),
                        NetworkProverStatus::Submitted { handle, metadata },
                    );
                    return Ok(ProofAggregationStatus::ProofGenerationInProgress);
                }
                Err(e) => {
                    tracing::error!("Network proof for slot hash {} failed: {:?}", slot_hash, e);
                    proof_statuses.insert(
                        slot_hash.clone(),
                        NetworkProverStatus::Err(anyhow::anyhow!("Network proving failed: {}", e)),
                    );
                    return Err(anyhow::anyhow!(
                        "Network proving failed for {}: {}",
                        slot_hash,
                        e
                    ));
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

        let public_data = AggregatedProofPublicData::from_block_proofs(
            &block_proofs_data,
            genesis_state_root.clone(),
        );

        tracing::trace!(%public_data, "generating aggregate proof");

        // Drop the read lock before submitting to the outer network.
        drop(proof_statuses);

        let outer_handle = self.outer_vm.add_hint_and_submit(&public_data).await?;

        let serialized_aggregated_proof = tokio::time::timeout(self.outer_proof_timeout, async {
            loop {
                match self.outer_vm.poll(&outer_handle).await {
                    Ok(Some(proof_bytes)) => {
                        break Ok(SerializedAggregatedProof {
                            raw_aggregated_proof: proof_bytes,
                        });
                    }
                    Ok(None) => {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                    Err(e) => {
                        break Err(anyhow::anyhow!("Outer network proving failed: {}", e));
                    }
                }
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Outer network proving timed out after {:?}",
                self.outer_proof_timeout
            )
        })??;

        let mut tracker = self.tracker.write().await;
        for slot_hash in block_header_hashes {
            tracker.remove(slot_hash);
        }

        Ok(ProofAggregationStatus::Success(serialized_aggregated_proof))
    }
}
