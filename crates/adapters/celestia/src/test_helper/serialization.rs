use sov_rollup_interface::da::{RelevantBlobs, RelevantProofs};

use crate::da_service::{extract_relevant_blobs, get_extraction_proof};
use crate::types::{BlobWithSender, FilteredCelestiaBlock, NamespaceBoundaryProof};
use crate::verifier::proofs::BlobProof;

type CelestiaRelevantProofs = RelevantProofs<Vec<BlobProof>, Option<NamespaceBoundaryProof>>;

pub(crate) async fn test_block_serialization(block: FilteredCelestiaBlock) {
    // Block itself
    let serialized_bincode_block =
        bincode::serialize(&block).expect("block bincode serialization failed");
    let deserialize_bincode_block =
        bincode::deserialize::<FilteredCelestiaBlock>(&serialized_bincode_block)
            .expect("block bincode deserialization failed");
    assert_eq!(block, deserialize_bincode_block);

    let serialized_risc0_block =
        risc0_zkvm::serde::to_vec(&block).expect("block risc0 serialization failed");
    let deserialized_risc0_block: FilteredCelestiaBlock =
        risc0_zkvm::serde::from_slice(&serialized_risc0_block)
            .expect("block risc0 deserialization failed");
    assert_eq!(block, deserialized_risc0_block);

    // Relevant blobs
    let relevant_blobs = extract_relevant_blobs(&block);

    let serialized_bincode_relevant_blobs =
        bincode::serialize(&relevant_blobs).expect("relevant blobs bincode serialization failed");
    let deserialized_bincode_relevant_blobs: RelevantBlobs<BlobWithSender> =
        bincode::deserialize(&serialized_bincode_relevant_blobs)
            .expect("relevant blobs bincode deserialization failed");
    assert_eq!(
        relevant_blobs.batch_blobs,
        deserialized_bincode_relevant_blobs.batch_blobs
    );
    assert_eq!(
        relevant_blobs.proof_blobs,
        deserialized_bincode_relevant_blobs.proof_blobs
    );

    let serialized_risc0_relevant_blobs = risc0_zkvm::serde::to_vec(&relevant_blobs)
        .expect("relevant blobs risc0 serialization failed");
    let deserialized_risc0_relevant_blobs: RelevantBlobs<BlobWithSender> =
        risc0_zkvm::serde::from_slice(&serialized_risc0_relevant_blobs)
            .expect("relevant blobs risc0 deserialization failed");

    assert_eq!(
        relevant_blobs.batch_blobs,
        deserialized_risc0_relevant_blobs.batch_blobs
    );
    assert_eq!(
        relevant_blobs.proof_blobs,
        deserialized_risc0_relevant_blobs.proof_blobs
    );

    // Extraction proof
    let extraction_proof = get_extraction_proof(&block, &relevant_blobs);

    let serialized_bincode_extraction_proof = bincode::serialize(&extraction_proof)
        .expect("extraction proof bincode serialization failed");
    let deserialized_bincode_extraction_proof: CelestiaRelevantProofs =
        bincode::deserialize(&serialized_bincode_extraction_proof)
            .expect("extraction proof bincode deserialization failed");
    compare_ext_proof(&extraction_proof, &deserialized_bincode_extraction_proof);

    let serialized_risc0_extraction_proof = risc0_zkvm::serde::to_vec(&extraction_proof)
        .expect("extraction proof risc0 serialization failed");
    let deserialized_risc0_extraction_proof: CelestiaRelevantProofs =
        risc0_zkvm::serde::from_slice(&serialized_risc0_extraction_proof)
            .expect("extraction proof risc0 deserialization failed");
    compare_ext_proof(&extraction_proof, &deserialized_risc0_extraction_proof);
}

fn compare_ext_proof(expected: &CelestiaRelevantProofs, deserialized: &CelestiaRelevantProofs) {
    assert_eq!(
        expected.proof.inclusion_proof,
        deserialized.proof.inclusion_proof
    );
    assert_eq!(
        expected.proof.completeness_proof,
        deserialized.proof.completeness_proof
    );
    assert_eq!(
        expected.batch.inclusion_proof,
        deserialized.batch.inclusion_proof
    );
    assert_eq!(
        expected.batch.completeness_proof,
        deserialized.batch.completeness_proof
    );
}
