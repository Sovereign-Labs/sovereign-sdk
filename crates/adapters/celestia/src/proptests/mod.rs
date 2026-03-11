mod cases;
use std::panic::{catch_unwind, AssertUnwindSafe};

use celestia_types::state::AddressTrait;
use nmt_rs::nmt_proof::NamespaceProof as NmtNamespaceProof;
use proptest::prelude::*;
use sov_rollup_interface::da::{BlobReaderTrait, DaVerifier};

use crate::da_service::{extract_relevant_blobs, get_extraction_proof};
use crate::test_support::{
    assert_subproof_start_indices_align, rollup_params, signer_for_index, NS_BATCH,
};
use crate::types::{
    FilteredCelestiaBlock, IncompleteNamespaceError, NamespaceBoundaryProof, NamespaceType,
    NamespaceValidationError, ProofError, ValidationError,
};
use crate::verifier::proofs::BlobProof;
use crate::verifier::CelestiaVerifier;

use cases::{
    mid_row_case_strategy, multi_row_case_strategy, single_row_absence_case_strategy,
    single_row_mid_row_case_strategy, zero_row_absence_case_strategy,
};

fn assert_err_without_panic(
    verifier: &CelestiaVerifier,
    block: &FilteredCelestiaBlock,
    relevant_blobs: &sov_rollup_interface::da::RelevantBlobs<crate::types::BlobWithSender>,
    proofs: sov_rollup_interface::da::RelevantProofs<
        Vec<BlobProof>,
        Option<NamespaceBoundaryProof>,
    >,
    context: &str,
) {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        verifier.verify_relevant_tx_list(&block.header, relevant_blobs, proofs)
    }));
    assert!(outcome.is_ok(), "verifier panicked for {context}");
    assert!(
        outcome.expect("checked above").is_err(),
        "expected verifier error for {context}"
    );
}

proptest! {
    #[test]
    fn proptest_blob_extraction_and_data_integrity(case in multi_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(relevant_blobs.batch_blobs.len(), case.batch_blob_shares.len());
        prop_assert!(relevant_blobs.proof_blobs.is_empty());

        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }

        for (idx, blob) in relevant_blobs.batch_blobs.iter().enumerate() {
            let expected_seed = case.seed.wrapping_add(idx as u8);
            let expected_signer = signer_for_index(case.seed, idx);
            let data = blob.verified_data();
            let sender = blob.sender();

            prop_assert!(!data.is_empty());
            prop_assert_eq!(data[0], expected_seed);
            prop_assert_eq!(sender.as_ref(), expected_signer.id_ref().as_ref());
        }
    }

    #[test]
    fn proptest_empty_blobs_fail_verification(case in multi_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        prop_assert!(!proofs.batch.inclusion_proof.is_empty());
        let verifier = CelestiaVerifier::new(rollup_params());
        relevant_blobs.batch_blobs.clear();

        prop_assert!(verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, proofs).is_err());
    }

    #[test]
    fn proptest_multi_blob_full_verification(case in multi_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        prop_assert_eq!(relevant_blobs.batch_blobs.len(), case.batch_blob_shares.len());
        prop_assert!(relevant_blobs.proof_blobs.is_empty());
        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }
        let proofs = get_extraction_proof(&block, &relevant_blobs);
        assert_subproof_start_indices_align(&block, "batch", &proofs.batch.inclusion_proof);
        assert_subproof_start_indices_align(&block, "proof", &proofs.proof.inclusion_proof);
        let result = CelestiaVerifier::new(rollup_params())
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "multi-blob verification failed with {:?} for case: {:?}",
            result.err(),
            case
        );
    }

    // Multi-candidate empty-namespace ambiguity is covered by deterministic tests in
    // `da_service/tests.rs` and `verifier/mod.rs`, not by generated end-to-end ODS layouts.
    #[test]
    fn proptest_single_row_empty_namespace_with_boundary_proof_verifies(
        case in single_row_absence_case_strategy()
    ) {
        let block = case.build_block();
        let candidate_rows = block.rollup_batch_data.data.rows();
        let namespace_row_root_count = block.header.get_row_roots_for_namespace(NS_BATCH).count();
        let relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(candidate_rows.len(), 1);
        prop_assert_eq!(namespace_row_root_count, 1);
        prop_assert!(candidate_rows[0].shares.is_empty());
        prop_assert!(relevant_blobs.batch_blobs.is_empty());
        prop_assert!(relevant_blobs.proof_blobs.is_empty());

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        prop_assert!(proofs.batch.inclusion_proof.is_empty());
        prop_assert!(proofs.batch.completeness_proof.is_some());

        let result = CelestiaVerifier::new(rollup_params())
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "single-row empty namespace should verify, got {:?} for case {:?}",
            result.err(),
            case
        );
    }

    #[test]
    fn proptest_mid_row_full_verification(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(relevant_blobs.batch_blobs.len(), 1);
        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        assert_subproof_start_indices_align(&block, "batch", &proofs.batch.inclusion_proof);
        assert_subproof_start_indices_align(&block, "proof", &proofs.proof.inclusion_proof);

        let result = CelestiaVerifier::new(rollup_params())
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "mid-row verification failed with {:?} for case: {:?}",
            result.err(),
            case
        );
    }

    #[test]
    fn proptest_single_row_mid_row_full_verification(case in single_row_mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(relevant_blobs.batch_blobs.len(), 1);
        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        assert_subproof_start_indices_align(&block, "batch", &proofs.batch.inclusion_proof);
        assert_subproof_start_indices_align(&block, "proof", &proofs.proof.inclusion_proof);

        let result = CelestiaVerifier::new(rollup_params())
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "single-row mid-row verification failed with {:?} for case: {:?}",
            result.err(),
            case
        );
    }

    #[test]
    fn proptest_mutated_start_share_idx_returns_err_without_panic(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        proofs.batch.inclusion_proof[0].range_proofs[0].start_share_idx =
            proofs.batch.inclusion_proof[0].range_proofs[0]
                .start_share_idx
                .saturating_add(1);

        assert_err_without_panic(
            &CelestiaVerifier::new(rollup_params()),
            &block,
            &relevant_blobs,
            proofs,
            "mutated start_share_idx",
        );
    }

    #[test]
    fn proptest_mutated_range_proof_order_returns_err_without_panic(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        let range_proofs = &mut proofs.batch.inclusion_proof[0].range_proofs;
        prop_assume!(range_proofs.len() > 1);
        range_proofs.reverse();

        assert_err_without_panic(
            &CelestiaVerifier::new(rollup_params()),
            &block,
            &relevant_blobs,
            proofs,
            "mutated range proof order",
        );
    }

    #[test]
    fn proptest_mutated_boundary_range_returns_err_without_panic(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        for blob in &mut relevant_blobs.batch_blobs {
            blob.advance(blob.total_len());
        }

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        let Some(boundary) = proofs.batch.completeness_proof.as_mut() else {
            prop_assume!(false);
            return Ok(());
        };

        match &mut *boundary.last_share_proof {
            NmtNamespaceProof::PresenceProof { proof, .. }
            | NmtNamespaceProof::AbsenceProof { proof, .. } => {
                proof.range.start = proof.range.start.saturating_add(1);
                proof.range.end = proof.range.end.saturating_add(1);
            }
        }

        assert_err_without_panic(
            &CelestiaVerifier::new(rollup_params()),
            &block,
            &relevant_blobs,
            proofs,
            "mutated boundary range",
        );
    }

    #[test]
    fn proptest_zero_candidate_rows_early_return(case in zero_row_absence_case_strategy()) {
        let block = case.build_block();
        let namespace_row_root_count = block.header.get_row_roots_for_namespace(NS_BATCH).count();
        let relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(namespace_row_root_count, 0);
        prop_assert!(relevant_blobs.batch_blobs.is_empty());
        prop_assert!(relevant_blobs.proof_blobs.is_empty());

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        let result = CelestiaVerifier::new(rollup_params())
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "zero candidate rows with empty blobs should verify, got {:?}",
            result.err()
        );
    }

    #[test]
    fn proptest_single_row_absence_without_boundary_proof_rejected(
        case in single_row_absence_case_strategy()
    ) {
        let block = case.build_block();
        let candidate_rows = block.rollup_batch_data.data.rows();
        let relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(candidate_rows.len(), 1);
        prop_assert!(relevant_blobs.batch_blobs.is_empty());

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        prop_assert!(proofs.batch.completeness_proof.is_some());
        proofs.batch.completeness_proof = None;

        let err = CelestiaVerifier::new(rollup_params())
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs)
            .expect_err("missing boundary proof should fail");
        prop_assert!(
            matches!(
                err,
                ValidationError::NamespaceValidationError {
                    namespace: NamespaceType::Batch,
                    error: NamespaceValidationError::IncompleteNamespace(
                        IncompleteNamespaceError::ProofError(ProofError::Missing)
                    ),
                }
            ),
            "unexpected verifier error: {:?}",
            err
        );
    }
}
