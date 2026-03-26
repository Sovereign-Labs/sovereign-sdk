//! Core aggregation circuit logic for verifying and chaining inner proofs
//! into an [`AggregatedProofPublicData`].

use core::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::common::{AggregatedProofWitness, DeferredProofInput};
use super::{AggregatedProofPublicData, CodeCommitmentHash};
use crate::common::SlotNumber;
use crate::da::{BlockHeaderTrait, DaSpec};
use crate::zk::{StateTransitionPublicData, ZkVerifier, ZkvmGuest};

struct BoundaryData<Hash, Root> {
    slot_hash: Hash,
    state_root: Root,
    slot_number: SlotNumber,
}

struct VerifiedProofData<Address, Hash, Root> {
    initial_boundary: BoundaryData<Hash, Root>,
    final_boundary: BoundaryData<Hash, Root>,
    rewarded_addresses: Vec<Address>,
}

type VerifyResult<Address, Da, Root> = VerifiedProofData<Address, <Da as DaSpec>::SlotHash, Root>;

/// Runs the aggregation circuit: reads a witness from the host, verifies inner
/// proofs and an optional previous outer proof, checks DA and state-root
/// continuity, then commits [`AggregatedProofPublicData`] as the public output.
pub fn run_aggregation_program<Address, Da, Root, V, G>(
    inner_vkey_hash: CodeCommitmentHash,
    guest: G,
) where
    Address: Clone + Serialize + DeserializeOwned,
    Da: DaSpec,
    Root: Clone + Debug + PartialEq + Serialize + DeserializeOwned,
    V: ZkVerifier<CodeCommitment = CodeCommitmentHash>,
    G: ZkvmGuest<Verifier = V>,
{
    let witness = guest.read_from_host::<AggregatedProofWitness<Da>>();

    let proof_inputs = witness.proof_inputs;
    let outer_vkey_hash = witness.outer_vkey_hash;
    let prev_outer_proof_witness = witness.prev_outer_proof_witness;

    // Verify the previous aggregation proof if one exists. On the first aggregation
    // after genesis, there is no predecessor, the chain starts here.
    let previous_public_data = prev_outer_proof_witness.map(|prev_outer_proof_witness| {
        V::verify::<AggregatedProofPublicData<Address, Da, Root>>(
            &prev_outer_proof_witness.public_values,
            &outer_vkey_hash,
        )
        .unwrap_or_else(|error| panic!("Failed to verify aggregated proof: {error:?}"))
    });

    let verified_proof_data: VerifyResult<Address, Da, Root> =
        verify_proof_chain::<Address, Da, Root, V>(
            proof_inputs,
            inner_vkey_hash,
            previous_public_data.as_ref(),
        );

    let VerifiedProofData {
        initial_boundary,
        final_boundary,
        rewarded_addresses,
    } = verified_proof_data;

    // Propagate the genesis state root forward through recursive aggregations.
    // For the very first aggregation, the genesis root is the initial state root
    // of the first inner proof (i.e. the state root at chain genesis).
    let genesis_state_root = previous_public_data
        .as_ref()
        .map(|public_data| public_data.genesis_state_root.clone())
        .unwrap_or_else(|| initial_boundary.state_root.clone());

    let aggregated_public_data = AggregatedProofPublicData::<Address, Da, Root> {
        initial_slot_number: initial_boundary.slot_number,
        final_slot_number: final_boundary.slot_number,
        genesis_state_root,
        initial_state_root: initial_boundary.state_root,
        final_state_root: final_boundary.state_root,
        initial_slot_hash: initial_boundary.slot_hash,
        final_slot_hash: final_boundary.slot_hash,
        outer_vk_hash: outer_vkey_hash,
        rewarded_addresses,
    };

    // Commit the aggregated public data as this program's public output.
    // This is what external verifiers (and the next recursive aggregation) will see.
    guest.commit(&aggregated_public_data);
}

fn verify_proof_chain<Address, Da, Root, V>(
    proof_inputs: Vec<DeferredProofInput<Da>>,
    vkey_hash: CodeCommitmentHash,
    previous_agg_proof_public_data: Option<&AggregatedProofPublicData<Address, Da, Root>>,
) -> VerifyResult<Address, Da, Root>
where
    Address: Clone + Serialize + DeserializeOwned,
    Da: DaSpec,
    Root: Clone + Debug + PartialEq + Serialize + DeserializeOwned,
    V: ZkVerifier<CodeCommitment = CodeCommitmentHash>,
{
    assert!(
        !proof_inputs.is_empty(),
        "Aggregated proof must contain at least one proof input"
    );

    let mut expected_prev_slot_hash =
        previous_agg_proof_public_data.map(|public_data| public_data.final_slot_hash.clone());

    let mut expected_prev_state_root =
        previous_agg_proof_public_data.map(|public_data| public_data.final_state_root.clone());

    // We intentionally scope the output to the current set of inner proofs only.
    // The predecessor proof is verified for chain continuity, but its slot range
    // and rewards are not carried forward — each aggregation covers only the
    // proofs it directly verifies.
    let mut initial_boundary = None;
    let mut final_boundary = None;

    let mut rewarded_addresses = Vec::with_capacity(proof_inputs.len());

    for (index, proof_input) in proof_inputs.iter().enumerate() {
        let stf_public_data = V::verify::<StateTransitionPublicData<Address, Da, Root>>(
            &proof_input.public_values,
            &vkey_hash,
        )
        .unwrap_or_else(|error| panic!("Failed to verify inner proof: {error:?}"));

        let current_slot_number = SlotNumber::new(proof_input.da_block_header.height());

        // Verify DA block hash-chain continuity: each block's prev_hash must equal
        // the predecessor's hash. Also cross-check that the DA header hash matches
        // the slot_hash committed in the STF proof's public data, binding the DA
        // layer to the execution layer.
        {
            let da_block_header = &proof_input.da_block_header;
            let current_block_hash = da_block_header.hash();

            if let Some(expected_prev_slot_hash) = &expected_prev_slot_hash {
                assert_eq!(
                    expected_prev_slot_hash,
                    &da_block_header.prev_hash(),
                    "DA block chain broken at index {index}: prev_hash mismatch"
                );
            }

            assert_eq!(
                current_block_hash, stf_public_data.slot_hash,
                "Slot hash mismatch at index {index}: DA block header hash doesn't match public data"
            );
            expected_prev_slot_hash = Some(current_block_hash);
        }

        // Verify state root continuity: each proof's initial_state_root must equal
        // the predecessor's final_state_root, ensuring no gaps in the state
        // transition.
        {
            if let Some(expected_prev_state_root) = &expected_prev_state_root {
                assert_eq!(
                    expected_prev_state_root, &stf_public_data.initial_state_root,
                    "State root discontinuity at index {index}: previous final_state_root != current initial_state_root"
                );
            }

            expected_prev_state_root = Some(stf_public_data.final_state_root.clone());
        }

        // Record the boundary data for this aggregation's public output.
        // initial_boundary is captured only from the first proof; final_boundary
        // is overwritten on every iteration so it reflects the last proof.
        if initial_boundary.is_none() {
            initial_boundary = Some(BoundaryData {
                slot_hash: proof_input.da_block_header.hash(),
                state_root: stf_public_data.initial_state_root.clone(),
                slot_number: current_slot_number,
            });
        }

        rewarded_addresses.push(stf_public_data.prover_address.clone());
        final_boundary = Some(BoundaryData {
            slot_hash: proof_input.da_block_header.hash(),
            state_root: stf_public_data.final_state_root,
            slot_number: current_slot_number,
        });
    }

    let initial_boundary = initial_boundary.expect("proof_inputs is non-empty");
    let final_boundary = final_boundary.expect("proof_inputs is non-empty");

    VerifyResult::<Address, Da, Root> {
        initial_boundary,
        final_boundary,
        rewarded_addresses,
    }
}
