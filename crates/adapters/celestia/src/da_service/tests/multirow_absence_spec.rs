use super::multirow_absence_fixtures::{
    canonical_single_row_absence_fixture, synthetic_noncanonical_multirow_absence_fixture,
    synthetic_noncanonical_multirow_mixed_presence_fixture,
};
use super::*;

#[test]
fn empty_namespace_multicandidate_is_not_verified_because_divergence_from_spec() {
    let (block, rollup_params) = synthetic_noncanonical_multirow_absence_fixture();
    let root_count = block
        .header
        .get_row_roots_for_namespace(rollup_params.rollup_batch_namespace)
        .count();
    assert!(
        root_count > 1,
        "fixture should have more than one candidate row root"
    );

    let relevant_blobs = RelevantBlobs {
        batch_blobs: Vec::new(),
        proof_blobs: Vec::new(),
    };
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    let verifier = CelestiaVerifier::new(rollup_params);
    let err = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();
    assert!(
        err.to_string().contains("MissingBlobs"),
        "MissingBlobs should be in error={}",
        err
    );
}

#[test]
fn empty_namespace_multicandidate_rows_without_global_evidence_rejected() {
    let (block, rollup_params) = synthetic_noncanonical_multirow_absence_fixture();
    let root_count = block
        .header
        .get_row_roots_for_namespace(rollup_params.rollup_batch_namespace)
        .count();
    assert!(
        root_count > 1,
        "fixture should have more than one candidate row root"
    );

    let relevant_blobs = RelevantBlobs {
        batch_blobs: Vec::new(),
        proof_blobs: Vec::new(),
    };
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    assert!(
        relevant_proofs.batch.completeness_proof.is_some(),
        "fixture should include a per-row boundary proof, even though global evidence is missing"
    );

    let verifier = CelestiaVerifier::new(rollup_params);
    let error_with_boundary = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();
    assert!(
        error_with_boundary.to_string().contains("MissingBlobs"),
        "expected MissingBlobs with boundary proof present, got: {error_with_boundary}"
    );

    let mut relevant_proofs_without_boundary = get_extraction_proof(&block, &relevant_blobs);
    relevant_proofs_without_boundary.batch.completeness_proof = None;
    let error_without_boundary = verifier
        .verify_relevant_tx_list(
            &block.header,
            &relevant_blobs,
            relevant_proofs_without_boundary,
        )
        .unwrap_err();
    assert!(
        error_without_boundary.to_string().contains("MissingBlobs"),
        "expected MissingBlobs with boundary proof removed, got: {error_without_boundary}"
    );
}

#[test]
fn empty_namespace_single_candidate_row_with_boundary_proof_is_accepted() {
    let (block, rollup_params) = canonical_single_row_absence_fixture();
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Vec::new(),
        proof_blobs: Vec::new(),
    };
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    assert!(
        relevant_proofs.batch.completeness_proof.is_some(),
        "single-row absence should include a boundary proof"
    );

    let verifier = CelestiaVerifier::new(rollup_params);
    verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap();
}

#[test]
fn empty_namespace_single_candidate_row_without_boundary_proof_rejected() {
    let (block, rollup_params) = canonical_single_row_absence_fixture();
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Vec::new(),
        proof_blobs: Vec::new(),
    };
    let mut relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    assert!(
        relevant_proofs.batch.completeness_proof.is_some(),
        "single-row absence fixture should produce completeness proof"
    );
    relevant_proofs.batch.completeness_proof = None;

    let verifier = CelestiaVerifier::new(rollup_params);
    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();
    assert!(
        error.to_string().contains("ProofError(Missing)"),
        "expected ProofError(Missing), got: {error}"
    );
}

#[test]
fn empty_namespace_claim_rejected_during_proof_construction_when_candidate_row_contains_real_shares(
) {
    let (block, rollup_params) = synthetic_noncanonical_multirow_mixed_presence_fixture();
    let root_count = block
        .header
        .get_row_roots_for_namespace(rollup_params.rollup_batch_namespace)
        .count();
    assert!(
        root_count > 1,
        "fixture should have more than one candidate row root"
    );

    let honest_blobs = extract_relevant_blobs(&block);
    assert!(
        !honest_blobs.batch_blobs.is_empty(),
        "synthetic fixture should contain real extracted batch blobs"
    );

    // Adversarial claim: pretend namespace is empty.
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Vec::new(),
        proof_blobs: Vec::new(),
    };
    let error = build_extraction_proof(&block, &relevant_blobs).unwrap_err();
    assert!(
        matches!(
            error,
            crate::types::ExtractionProofError::MissingBlobsForSupportedNamespace { .. }
        ),
        "expected MissingBlobsForSupportedNamespace, got: {error}"
    );
}

#[test]
fn single_row_legacy_boundary_proof_flow_still_verifies() {
    let (block, rollup_params, signers) = with_rollup_batch_data::test_case();
    let root_count = block
        .header
        .get_row_roots_for_namespace(rollup_params.rollup_batch_namespace)
        .count();
    assert_eq!(
        root_count, 1,
        "fixture should have exactly one candidate row"
    );
    assert!(
        !signers.is_empty(),
        "fixture should contain at least one signer"
    );

    let mut relevant_blobs = extract_relevant_blobs(&block);
    apply_blob_readers(&mut relevant_blobs, read_full_blob, read_full_blob);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    let verifier = CelestiaVerifier::new(rollup_params);
    verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap();
}
