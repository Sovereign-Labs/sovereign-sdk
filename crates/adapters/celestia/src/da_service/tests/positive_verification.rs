//! Positive-path end-to-end verification and fixture regression coverage.

use super::*;

fn verification_for_correct_blocks(
    batch_processing_fn: BlobReader,
    proof_processing_fn: BlobReader,
) {
    let blocks = [
        with_rollup_batch_data::test_case(),
        with_rollup_proof_data::test_case(),
        without_rollup_batch_data::test_case(),
        with_several_small_rollup_batches::test_case(),
        with_several_medium_rollup_batches::test_case(),
        with_several_large_rollup_batches::test_case(),
        with_preceding_blobs_from_different_namespaces::test_case(),
        with_batch_and_proof_same_block::test_case(),
        with_namespace_padding::test_case(),
        with_parity_boundary_followed_by_namespace::test_case(),
        medium_from_devnet::test_case(),
        from_testnet::test_case(),
        from_testnet_no_shares::test_case(),
        with_mixed_v0_and_v1_blobs::test_case(),
        with_mixed_v0_and_v1_multi_v1_parity_boundary::test_case(),
        from_testnet_with_tail_padding::test_case(),
        from_mocha_shares_mismatch::test_case(),
        from_mocha_invalid_row_proof::test_case(),
        from_mocha_multi_candidate_rows_10261831::test_case(),
    ];

    for (block, rollup_params, signers) in blocks {
        let mut relevant_blobs = extract_relevant_blobs(&block);
        read_fixture_blobs(
            &mut relevant_blobs,
            signers,
            batch_processing_fn,
            proof_processing_fn,
        );

        let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);

        let verifier = CelestiaVerifier::new(rollup_params);

        verifier
            .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
            .unwrap();
    }
}

#[test]
fn verification_succeeds_for_correct_blocks() {
    let read_modes: [(&str, BlobReader, BlobReader); 9] = [
        ("none/none", read_no_blob, read_no_blob),
        ("full/full", read_full_blob, read_full_blob),
        ("full/none", read_full_blob, read_no_blob),
        ("none/full", read_no_blob, read_full_blob),
        ("full/single", read_full_blob, read_single_byte_blob),
        (
            "single/single",
            read_single_byte_blob,
            read_single_byte_blob,
        ),
        ("half/half", read_half_blob, read_half_blob),
        ("half/none", read_half_blob, read_no_blob),
        ("none/half", read_no_blob, read_half_blob),
    ];

    for (_, batch_reader, proof_reader) in read_modes {
        verification_for_correct_blocks(batch_reader, proof_reader);
    }
}

#[test]
fn regression_mocha_invalid_row_proof_minimal_read_mode() {
    verify_fixture_with_readers(
        from_mocha_invalid_row_proof::test_case(),
        read_single_byte_blob,
        read_single_byte_blob,
    );
}

#[test]
fn regression_mocha_invalid_row_proof_full_read_mode() {
    verify_fixture_with_readers(
        from_mocha_invalid_row_proof::test_case(),
        read_full_blob,
        read_full_blob,
    );
}

#[test]
fn proof_start_indexes_match_row_coordinates_for_valid_fixtures() {
    let fixtures = [
        with_rollup_batch_data::test_case(),
        with_namespace_padding::test_case(),
        from_mocha_invalid_row_proof::test_case(),
    ];

    for (block, _rollup_params, _signers) in fixtures {
        let mut relevant_blobs = extract_relevant_blobs(&block);
        apply_blob_readers(&mut relevant_blobs, read_full_blob, read_full_blob);

        let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
        assert_subproof_start_indices_align(
            &block,
            "batch",
            &relevant_proofs.batch.inclusion_proof,
        );
        assert_subproof_start_indices_align(
            &block,
            "proof",
            &relevant_proofs.proof.inclusion_proof,
        );
    }
}

#[test]
fn parity_boundary_fixture_contains_target_shape() {
    let block = with_parity_boundary_followed_by_namespace::filtered_block();
    let namespace =
        with_parity_boundary_followed_by_namespace::ROLLUP_PARAMS.rollup_batch_namespace;
    assert!(
        has_parity_boundary_followed_by_namespace(
            &block.rollup_batch_data.data,
            namespace,
            block.header.row_length(),
        ),
        "Fixture must contain a row ending at last non-parity share with parity right sibling and namespace continuation in next row",
    );
}

#[test]
fn mixed_multi_v1_parity_boundary_verification_survives_partial_reads() {
    let block = with_mixed_v0_and_v1_multi_v1_parity_boundary::filtered_block();
    let rollup_params = with_mixed_v0_and_v1_multi_v1_parity_boundary::ROLLUP_PARAMS;
    let verifier = CelestiaVerifier::new(rollup_params);
    let namespace = rollup_params.rollup_batch_namespace;
    let mut v0_starts = 0usize;
    let mut v1_starts = 0usize;
    for row in block.rollup_batch_data.data.rows() {
        for share in &row.shares {
            if share.is_parity() || crate::shares::is_tail_padding(share) {
                continue;
            }
            let Some(info_byte) = share.info_byte() else {
                continue;
            };
            if info_byte.is_sequence_start() {
                match info_byte.version() {
                    0 => v0_starts += 1,
                    1 => v1_starts += 1,
                    _ => {}
                }
            }
        }
    }

    assert!(
        has_parity_boundary_followed_by_namespace(
            &block.rollup_batch_data.data,
            namespace,
            block.header.row_length(),
        ),
        "Fixture should include parity boundary shape"
    );
    assert!(v0_starts > 0, "Fixture should include v0 blobs");
    assert!(v1_starts >= 2, "Fixture should include multiple v1 blobs");

    for read_mode in ["no_read", "single_byte", "full"] {
        let mut relevant_blobs = extract_relevant_blobs(&block);
        assert_eq!(
            relevant_blobs.proof_blobs.len(),
            0,
            "Fixture should only target batch namespace for this test"
        );
        assert!(
            relevant_blobs.batch_blobs.len() >= 2,
            "Fixture should extract multiple supported v1 blobs",
        );

        for blob in relevant_blobs.batch_blobs.iter_mut() {
            let total_len = blob.blob.total_len();
            match read_mode {
                "no_read" => {}
                "single_byte" => {
                    if total_len > 0 {
                        blob.blob.advance(1);
                    }
                }
                "full" => blob.blob.advance(total_len),
                _ => unreachable!("unexpected read mode"),
            }
        }

        let relevant_proofs = get_extraction_proof(&block, &relevant_blobs);
        assert!(
            relevant_proofs.batch.inclusion_proof.len() > relevant_blobs.batch_blobs.len(),
            "Expected skipped v0 blobs to contribute extra inclusion proofs (mode={read_mode})",
        );

        verifier
            .verify_relevant_tx_list(&block.header, &relevant_blobs, relevant_proofs)
            .unwrap_or_else(|err| {
                panic!("Mixed multi-v1 verification failed in mode={read_mode}: {err}")
            });
    }
}

#[test]
fn test_payload_can_be_read_back() -> anyhow::Result<()> {
    let cases = [
        (
            with_rollup_batch_data::test_case(),
            with_rollup_batch_data::get_payload(),
        ),
        (
            with_rollup_proof_data::test_case(),
            with_rollup_proof_data::get_payload(),
        ),
        (
            with_several_small_rollup_batches::test_case(),
            with_several_small_rollup_batches::get_payload(),
        ),
        (
            with_several_medium_rollup_batches::test_case(),
            with_several_medium_rollup_batches::get_payload(),
        ),
        (
            with_several_large_rollup_batches::test_case(),
            with_several_large_rollup_batches::get_payload(),
        ),
        (
            with_namespace_padding::test_case(),
            with_namespace_padding::get_payload(),
        ),
    ];

    let assert_payload = |blobs: &mut Vec<BlobWithSender>, expected_blobs: Vec<Vec<u8>>| {
        assert_eq!(blobs.len(), expected_blobs.len());
        for (actual_batch, expected_batch) in blobs.iter_mut().zip(expected_blobs.iter()) {
            let total_len = actual_batch.blob.total_len();
            assert_eq!(total_len, expected_batch.len());
            actual_batch.blob.advance(total_len);
            let full_data = actual_batch.blob.accumulator();

            assert_eq!(full_data, expected_batch);
        }
    };

    for ((block, _rollup_params, _signers), payload) in cases {
        let mut relevant_blobs = extract_relevant_blobs(&block);

        let expected_batches = payload.batches();
        assert_payload(&mut relevant_blobs.batch_blobs, expected_batches);

        let expected_proofs = payload.proofs();
        assert_payload(&mut relevant_blobs.proof_blobs, expected_proofs);
    }

    Ok(())
}
