#![no_main]

sp1_zkvm::entrypoint!(main);

use sov_celestia_adapter::types::{BlobWithSender, FilteredCelestiaBlock, NamespaceBoundaryProof};
use sov_celestia_adapter::types::Namespace;
use sov_celestia_adapter::verifier::proofs::BlobProof;
use sov_celestia_adapter::verifier::{CelestiaVerifier, RollupParams};
use sov_rollup_interface::da::{
    BlobReaderTrait, BlockHeaderTrait, DaVerifier, RelevantBlobs, RelevantProofs,
};

type CelestiaRelevantProofs = RelevantProofs<Vec<BlobProof>, Option<NamespaceBoundaryProof>>;
type CelestiaGuestOutput = ([u8; 32], u64, u64, u64, u64, u64);

const ROLLUP_PARAMS: RollupParams = RollupParams {
    rollup_batch_namespace: Namespace::const_v0([0, 0, 10, 117, 61, 127, 167, 56, 47, 69]),
    rollup_proof_namespace: Namespace::const_v0([115, 111, 118, 45, 116, 101, 115, 116, 45, 112]),
};

pub fn main() {
    println!("cycle-tracker-report-start: celestia_full_block");

    let block_bytes = sp1_zkvm::io::read_vec();
    let relevant_blobs: RelevantBlobs<BlobWithSender> = sp1_zkvm::io::read();
    let relevant_proofs: CelestiaRelevantProofs = sp1_zkvm::io::read();

    let block: FilteredCelestiaBlock =
        bincode::deserialize(&block_bytes).expect("valid filtered Celestia block");
    let header = block.header();
    let block_hash = *header.hash().inner();
    let verifier = CelestiaVerifier::new(ROLLUP_PARAMS);
    let output = guest_output(block_hash, block_bytes.len() as u64, &relevant_blobs);

    println!("cycle-tracker-report-start: celestia_verify");
    verifier
        .verify_relevant_tx_list(header, &relevant_blobs, relevant_proofs)
        .expect("Celestia verification should succeed");
    println!("cycle-tracker-report-end: celestia_verify");

    println!("cycle-tracker-report-end: celestia_full_block");

    sp1_zkvm::io::commit(&output);
}

fn guest_output(
    block_hash: [u8; 32],
    block_bytes: u64,
    relevant_blobs: &RelevantBlobs<BlobWithSender>,
) -> CelestiaGuestOutput {
    let batch_blob_count = relevant_blobs.batch_blobs.len() as u64;
    let proof_blob_count = relevant_blobs.proof_blobs.len() as u64;
    let batch_bytes = relevant_blobs
        .batch_blobs
        .iter()
        .map(BlobReaderTrait::total_len)
        .sum::<usize>() as u64;
    let proof_bytes = relevant_blobs
        .proof_blobs
        .iter()
        .map(BlobReaderTrait::total_len)
        .sum::<usize>() as u64;

    (
        block_hash,
        block_bytes,
        batch_blob_count,
        proof_blob_count,
        batch_bytes,
        proof_bytes,
    )
}
