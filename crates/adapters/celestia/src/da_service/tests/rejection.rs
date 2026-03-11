//! Negative-path rejection and malformed/missing-proof coverage.

use super::*;

#[test]
fn extraction_proof_fails_if_sender_changed() {
    // This is the preparation part, consider it as malicious native code:
    let mut block = with_rollup_batch_data::filtered_block();
    let addr_1 = CelestiaAddress::from_str(ADDR_1).unwrap();
    let addr_2 = CelestiaAddress::from_str(crate::test_helper::ADDR_2).unwrap();
    let addr_len = addr_1.as_ref().len();

    let serialized_ns_data = serde_json::to_string(&block.rollup_batch_data.data).unwrap();
    let row = block.rollup_batch_data.data.rows().first().unwrap();
    let share = row.shares.first().unwrap();
    // Save it to string for replacing it in JSON in the future.
    let serialized_share_before = serde_json::to_string(share).unwrap();

    let mut raw_share_1 = share.data().clone().to_vec();

    let add_pos = raw_share_1
        .windows(addr_len)
        .position(|window| window == addr_1.as_ref())
        .expect("Block should contain given address. Check source data");

    raw_share_1.splice(add_pos..add_pos + addr_len, addr_2.as_ref().iter().copied());

    let malicious_share = celestia_types::Share::from_raw(&raw_share_1).unwrap();
    let serialized_malicious_share = serde_json::to_string(&malicious_share).unwrap();

    let malicious_ns_data_json =
        serialized_ns_data.replace(&serialized_share_before, &serialized_malicious_share);
    let malicious_ns_data: NamespaceData = serde_json::from_str(&malicious_ns_data_json).unwrap();

    block.rollup_batch_data.data = malicious_ns_data;

    let relevant_blobs = extract_relevant_blobs(&block);
    let err = build_extraction_proof(&block, &relevant_blobs).unwrap_err();
    assert!(
        matches!(err, crate::types::ExtractionProofError::ProofSelfCheck(_)),
        "Actual error: {err}"
    );
}

#[test]
fn verification_fails_if_tx_missing() {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    let verifier = CelestiaVerifier::new(rollup_params);

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: Default::default(),
    };
    // give to verifier an empty transactions list
    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        matches!(
            error,
            crate::types::ValidationError::NamespaceValidationError {
                error: crate::types::NamespaceValidationError::InvalidBlobData(
                    crate::types::BlobDataError::MoreProofsThanBlobs
                ),
                ..
            }
        ),
        "Actual error: {error}"
    );
}

#[test]
fn extraction_proof_fails_for_empty_blob_list_when_supported_shares_exist() {
    let block = with_mixed_v0_and_v1_blobs::filtered_block();
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Default::default(),
        proof_blobs: Default::default(),
    };

    assert_missing_supported_namespace_error(&block, relevant_blobs);
}

#[test]
fn verification_fails_if_supported_namespace_has_empty_blob_list() {
    let block = with_rollup_batch_data::filtered_block();
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Default::default(),
        proof_blobs: Default::default(),
    };

    assert_missing_supported_namespace_error(&block, relevant_blobs);
}

#[test]
fn extraction_proof_rejects_out_of_bounds_blob_range() {
    let block = with_rollup_batch_data::filtered_block();
    let mut relevant_blobs = extract_relevant_blobs(&block);
    assert!(
        !relevant_blobs.batch_blobs.is_empty(),
        "fixture should contain at least one batch blob"
    );
    relevant_blobs.batch_blobs[0].range_in_namespace = 0..usize::MAX;

    let err = build_extraction_proof(&block, &relevant_blobs).unwrap_err();
    assert!(
        matches!(
            err,
            crate::types::ExtractionProofError::BlobRangeOutOfBounds { .. }
        ),
        "Actual error: {err}"
    );
}

#[test]
fn verification_fails_if_not_all_blobs_are_proven() {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);

    let mut relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
    // drop the proof for last batch
    relevant_proofs.batch.inclusion_proof.pop();

    let verifier = CelestiaVerifier::new(rollup_params);

    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        matches!(
            error,
            crate::types::ValidationError::NamespaceValidationError {
                error: crate::types::NamespaceValidationError::InvalidRowProof(
                    crate::types::RowProofError::ProofError(crate::types::ProofError::Missing)
                ),
                ..
            }
        ),
        "Actual error: {error}"
    );
}

#[test]
fn test_blobs_from_padded_namespace() {
    let block: FilteredCelestiaBlock = with_namespace_padding::filtered_block();
    let relevant_blobs = extract_relevant_blobs(&block);
    assert_eq!(relevant_blobs.batch_blobs.len(), 1);
    assert_eq!(relevant_blobs.proof_blobs.len(), 0);
}

#[test]
fn verification_for_padded_namespace() {
    let block: FilteredCelestiaBlock = with_namespace_padding::filtered_block();
    let rollup_params = with_namespace_padding::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    let verifier = CelestiaVerifier::new(rollup_params);

    verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap();
}

#[test]
fn verification_fails_if_there_is_less_blobs_than_proofs() {
    let block = with_rollup_batch_data::filtered_block();
    let rollup_params = with_rollup_batch_data::ROLLUP_PARAMS;

    let relevant_blobs = extract_relevant_blobs(&block);
    let mut relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    // push one extra blob proof
    relevant_proofs
        .batch
        .inclusion_proof
        .push(relevant_proofs.batch.inclusion_proof[0].clone());

    let verifier = CelestiaVerifier::new(rollup_params);

    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();

    assert!(
        error.to_string().contains("WrongStartShareIndex"),
        "Actual error: {error}"
    );
}

#[test]
fn verification_fails_for_incorrect_namespace() {
    let block = with_rollup_proof_data::filtered_block();

    let relevant_blobs = extract_relevant_blobs(&block);
    let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

    // create a verifier with a different namespace than the da_service
    let verifier = CelestiaVerifier::new(RollupParams {
        rollup_proof_namespace: Namespace::new_v0(b"abc").unwrap(),
        rollup_batch_namespace: Namespace::new_v0(b"xyz").unwrap(),
    });

    let error = verifier
        .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("InvalidRowProof(ProofError(Invalid(InvalidRoot)))"),
        "Actual error: {error}"
    );
}
