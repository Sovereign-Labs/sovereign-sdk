use std::panic::{catch_unwind, AssertUnwindSafe};

use nmt_rs::nmt_proof::NamespaceProof as NmtNamespaceProof;
use sov_rollup_interface::da::RelevantProofs;

use super::*;
use crate::test_support::{shift_namespace_proof_range, skipped_proof_index};
use crate::verifier::proofs::BlobProof;

#[derive(Debug)]
struct AdversarialCase {
    block: FilteredCelestiaBlock,
    params: RollupParams,
    blobs: RelevantBlobs<BlobWithSender>,
    proofs: RelevantProofs<Vec<BlobProof>, Option<NamespaceBoundaryProof>>,
}

fn load_case(
    fixture: (FilteredCelestiaBlock, RollupParams, Vec<CelestiaAddress>),
) -> AdversarialCase {
    let (block, params, _) = fixture;
    let mut blobs = extract_relevant_blobs(&block);
    apply_blob_readers(&mut blobs, read_full_blob, read_full_blob);

    let proofs = get_extraction_proof(&block, &blobs);
    CelestiaVerifier::new(params)
        .verify_relevant_tx_list(&block.header, &blobs, proofs.clone())
        .expect("baseline block should verify");

    AdversarialCase {
        block,
        params,
        blobs,
        proofs,
    }
}

fn skipped_batch_proof_index(case: &AdversarialCase) -> usize {
    skipped_proof_index(&case.proofs.batch.inclusion_proof)
        .expect("expected at least one skipped blob proof")
}

fn expect_verification_error(
    fixture: (FilteredCelestiaBlock, RollupParams, Vec<CelestiaAddress>),
    patterns: &[&str],
    mutate: impl FnOnce(&mut AdversarialCase),
) {
    let mut case = load_case(fixture);
    mutate(&mut case);

    let result = catch_unwind(AssertUnwindSafe(|| {
        CelestiaVerifier::new(case.params).verify_relevant_tx_list(
            &case.block.header,
            &case.blobs,
            case.proofs,
        )
    }));

    match result {
        Ok(Ok(_)) => panic!("expected verification to fail, but it succeeded"),
        Ok(Err(err)) => {
            let message = err.to_string();
            assert!(
                patterns.is_empty() || patterns.iter().any(|p| message.contains(p)),
                "Expected error to contain one of {patterns:?}, got: {message}",
            );
        }
        Err(_) => panic!("verification panicked; expected Err"),
    }
}

#[test]
fn verification_fails_if_blob_order_swapped() {
    expect_verification_error(
        with_several_small_rollup_batches::test_case(),
        &["NonMatchingShare"],
        |case| {
            assert!(case.blobs.batch_blobs.len() >= 2);
            case.blobs.batch_blobs.swap(0, 1);
        },
    );
}

#[test]
fn verification_fails_if_blob_duplicated() {
    expect_verification_error(
        with_rollup_batch_data::test_case(),
        &["WrongStartShareIndex"],
        |case| {
            let blob = case.blobs.batch_blobs[0].clone();
            let proof = case.proofs.batch.inclusion_proof[0].clone();
            case.blobs.batch_blobs.insert(1, blob);
            case.proofs.batch.inclusion_proof.insert(1, proof);
        },
    );
}

#[test]
fn verification_fails_if_fake_blob_inserted() {
    expect_verification_error(
        with_several_small_rollup_batches::test_case(),
        &["WrongSender"],
        |case| {
            let mut forged_blob = case.blobs.batch_blobs[1].clone();
            forged_blob.sender =
                CelestiaAddress::from_str(crate::test_helper::ADDR_2).expect("valid test address");
            let proof = case.proofs.batch.inclusion_proof[1].clone();
            case.blobs.batch_blobs.insert(1, forged_blob);
            case.proofs.batch.inclusion_proof.insert(1, proof);
        },
    );
}

#[test]
fn verification_fails_if_left_boundary_missing() {
    expect_verification_error(
        with_several_small_rollup_batches::test_case(),
        &["ProofError(Missing)"],
        |case| {
            case.proofs.batch.inclusion_proof.remove(0);
        },
    );
}

#[test]
fn verification_fails_if_right_boundary_missing() {
    expect_verification_error(
        with_namespace_padding::test_case(),
        &["ProofError(Missing)"],
        |case| {
            assert!(case.proofs.batch.completeness_proof.is_some());
            case.proofs.batch.completeness_proof = None;
        },
    );
}

#[test]
fn verification_fails_if_right_boundary_missing_blobs() {
    expect_verification_error(
        with_namespace_padding::test_case(),
        &["MissingBlobs", "Corrupted"],
        |case| {
            let boundary = case
                .proofs
                .batch
                .completeness_proof
                .as_mut()
                .expect("expected completeness proof to be present");
            shift_namespace_proof_range(&mut boundary.last_share_proof, true);
        },
    );
}

#[test]
fn verification_fails_if_gap_between_blobs() {
    expect_verification_error(
        with_several_small_rollup_batches::test_case(),
        &["WrongStartShareIndex"],
        |case| {
            case.proofs.batch.inclusion_proof[1].range_proofs[0].start_share_idx =
                case.proofs.batch.inclusion_proof[1].range_proofs[0]
                    .start_share_idx
                    .saturating_add(1);
        },
    );
}

#[test]
fn verification_fails_if_supported_blob_proves_wrong_share_count() {
    expect_verification_error(
        with_rollup_batch_data::test_case(),
        &["WrongNumberOfShares"],
        |case| {
            let first_range = &mut case.proofs.batch.inclusion_proof[0].range_proofs[0];
            let duplicate_share = first_range
                .shares
                .first()
                .expect("expected at least one proven share")
                .clone();
            first_range.shares.push(duplicate_share);
        },
    );
}

#[test]
fn verification_fails_if_start_index_manipulated() {
    expect_verification_error(
        with_rollup_batch_data::test_case(),
        &["MissingBlobs"],
        |case| {
            case.proofs.batch.inclusion_proof[0].range_proofs[0].start_share_idx =
                case.proofs.batch.inclusion_proof[0].range_proofs[0]
                    .start_share_idx
                    .saturating_add(1);
        },
    );
}

#[test]
fn verification_fails_for_wrong_namespace_proof() {
    expect_verification_error(
        with_rollup_batch_data::test_case(),
        &["InvalidRoot"],
        |case| {
            case.params.rollup_batch_namespace = Namespace::new_v0(b"xyz").unwrap();
            case.params.rollup_proof_namespace = Namespace::new_v0(b"abc").unwrap();
        },
    );
}

#[test]
fn verification_fails_if_proofs_reordered() {
    expect_verification_error(
        with_several_small_rollup_batches::test_case(),
        &["MissingBlobs"],
        |case| {
            case.proofs.batch.inclusion_proof.reverse();
        },
    );
}

#[test]
fn verification_fails_if_proof_blob_order_swapped() {
    expect_verification_error(
        with_batch_and_proof_same_block::test_case(),
        &["NonMatchingShare"],
        |case| {
            assert!(case.blobs.proof_blobs.len() >= 2);
            case.blobs.proof_blobs.swap(0, 1);
        },
    );
}

#[test]
fn verification_fails_if_fake_proof_blob_inserted() {
    expect_verification_error(
        with_rollup_proof_data::test_case(),
        &["WrongStartShareIndex"],
        |case| {
            let proof = case.proofs.proof.inclusion_proof[0].clone();
            case.proofs.proof.inclusion_proof.push(proof);
        },
    );
}

#[test]
fn verification_fails_if_proof_right_boundary_missing_blobs() {
    expect_verification_error(
        with_batch_and_proof_same_block::test_case(),
        &["MissingBlobs", "Corrupted"],
        |case| {
            let boundary = case
                .proofs
                .proof
                .completeness_proof
                .as_mut()
                .expect("expected completeness proof to be present");
            shift_namespace_proof_range(&mut boundary.last_share_proof, true);
        },
    );
}

#[test]
fn verification_fails_if_boundary_proof_row_alignment_is_manipulated() {
    expect_verification_error(
        from_testnet_with_tail_padding::test_case(),
        &["MissingBlobs", "Corrupted"],
        |case| {
            let boundary = case
                .proofs
                .batch
                .completeness_proof
                .as_mut()
                .expect("expected completeness proof to be present");
            shift_namespace_proof_range(&mut boundary.last_share_proof, false);
        },
    );
}

#[test]
fn verification_handles_out_of_bounds_row_index_without_panic() {
    expect_verification_error(with_rollup_batch_data::test_case(), &[], |case| {
        case.proofs.batch.inclusion_proof[0].range_proofs[0].start_share_idx = usize::MAX;
    });
}

#[test]
fn verification_fails_if_skipped_blob_proof_range_is_corrupted() {
    expect_verification_error(
        with_mixed_v0_and_v1_blobs::test_case(),
        &["Invalid"],
        |case| {
            let skipped_idx = skipped_batch_proof_index(case);
            let skipped_proof = &mut case.proofs.batch.inclusion_proof[skipped_idx];
            match &mut *skipped_proof.range_proofs[0].proof {
                NmtNamespaceProof::PresenceProof { proof, .. }
                | NmtNamespaceProof::AbsenceProof { proof, .. } => {
                    proof.range.start = proof.range.start.saturating_add(1);
                    proof.range.end = proof.range.end.saturating_add(1);
                }
            }
        },
    );
}

#[test]
fn verification_fails_if_tail_padding_share_payload_is_non_zero() {
    expect_verification_error(
        from_testnet_with_tail_padding::test_case(),
        &["NonMatchingShare"],
        |case| {
            let skipped_idx = skipped_batch_proof_index(case);
            let first_share =
                case.proofs.batch.inclusion_proof[skipped_idx].range_proofs[0].shares[0].clone();
            let mut raw_share = first_share.data().to_vec();
            let last_idx = raw_share
                .len()
                .checked_sub(1)
                .expect("tail padding share should not be empty");
            raw_share[last_idx] ^= 0x01;
            let malformed_share =
                celestia_types::Share::from_raw(&raw_share).expect("mutated share should parse");
            case.proofs.batch.inclusion_proof[skipped_idx].range_proofs[0].shares[0] =
                malformed_share;
        },
    );
}
