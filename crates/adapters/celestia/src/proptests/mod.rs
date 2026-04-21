mod cases;
use std::panic::{catch_unwind, AssertUnwindSafe};

use celestia_types::state::AddressTrait;
use proptest::prelude::*;
use sov_rollup_interface::da::{BlobReaderTrait, DaVerifier, RelevantBlobs, RelevantProofs};

use crate::da_service::{extract_relevant_blobs, get_extraction_proof};
use crate::test_helper::files::{
    with_batch_and_proof_same_block, with_mixed_v0_and_v1_blobs,
    with_mixed_v0_and_v1_multi_v1_parity_boundary, with_rollup_proof_data,
};
use crate::test_support::{
    assert_subproof_start_indices_align, rollup_params, shift_namespace_proof_range,
    signer_for_index, skipped_proof_index, synthetic_noncanonical_multirow_absence_fixture,
    NS_BATCH,
};
use crate::types::{
    BlobWithSender, FilteredCelestiaBlock, IncompleteNamespaceError, NamespaceBoundaryProof,
    NamespaceType, NamespaceValidationError, ProofError, ValidationError,
};
use crate::verifier::proofs::BlobProof;
use crate::verifier::{CelestiaVerifier, RollupParams};

use cases::{
    mid_row_case_strategy, mixed_version_fixture_strategy, multi_row_case_strategy,
    proof_fixture_strategy, read_mode_strategy, single_row_absence_case_strategy,
    single_row_mid_row_case_strategy, zero_row_absence_case_strategy, MixedVersionFixtureKind,
    ProofFixtureKind, ReadMode,
};

type BlobSet = RelevantBlobs<BlobWithSender>;
type ProofSet = RelevantProofs<Vec<BlobProof>, Option<NamespaceBoundaryProof>>;

fn apply_read_mode(blob: &mut BlobWithSender, mode: ReadMode) {
    let total_len = blob.total_len();
    match mode {
        ReadMode::None => {}
        ReadMode::SingleByte => {
            if total_len > 0 {
                blob.advance(1);
            }
        }
        ReadMode::Full => {
            blob.advance(total_len);
        }
    }
}

fn extract_with_read_modes(
    block: &FilteredCelestiaBlock,
    batch_mode: ReadMode,
    proof_mode: ReadMode,
) -> BlobSet {
    let mut relevant_blobs = extract_relevant_blobs(block);
    for blob in &mut relevant_blobs.batch_blobs {
        apply_read_mode(blob, batch_mode);
    }
    for blob in &mut relevant_blobs.proof_blobs {
        apply_read_mode(blob, proof_mode);
    }
    relevant_blobs
}

fn load_mixed_version_fixture(
    kind: MixedVersionFixtureKind,
) -> (FilteredCelestiaBlock, RollupParams) {
    match kind {
        MixedVersionFixtureKind::Basic => {
            let (block, params, _) = with_mixed_v0_and_v1_blobs::test_case();
            (block, params)
        }
        MixedVersionFixtureKind::ParityBoundary => {
            let (block, params, _) = with_mixed_v0_and_v1_multi_v1_parity_boundary::test_case();
            (block, params)
        }
    }
}

fn load_proof_fixture(kind: ProofFixtureKind) -> (FilteredCelestiaBlock, RollupParams) {
    match kind {
        ProofFixtureKind::ProofOnly => {
            let (block, params, _) = with_rollup_proof_data::test_case();
            (block, params)
        }
        ProofFixtureKind::BatchAndProof => {
            let (block, params, _) = with_batch_and_proof_same_block::test_case();
            (block, params)
        }
    }
}

fn assert_err_without_panic(
    verifier: &CelestiaVerifier,
    block: &FilteredCelestiaBlock,
    relevant_blobs: &BlobSet,
    proofs: ProofSet,
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
        let relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::None);

        prop_assert_eq!(relevant_blobs.batch_blobs.len(), case.batch_blob_shares.len());
        prop_assert!(relevant_blobs.proof_blobs.is_empty());

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
        let mut relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::None);

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        prop_assert!(!proofs.batch.inclusion_proof.is_empty());
        let verifier = CelestiaVerifier::new(rollup_params());
        relevant_blobs.batch_blobs.clear();

        prop_assert!(verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, proofs).is_err());
    }

    #[test]
    fn proptest_multi_blob_full_verification(case in multi_row_case_strategy()) {
        let block = case.build_block();
        let relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::Full);
        prop_assert_eq!(relevant_blobs.batch_blobs.len(), case.batch_blob_shares.len());
        prop_assert!(relevant_blobs.proof_blobs.is_empty());
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
        let relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::None);

        prop_assert_eq!(relevant_blobs.batch_blobs.len(), 1);

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
        let relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::None);

        prop_assert_eq!(relevant_blobs.batch_blobs.len(), 1);

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
        let relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::None);

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
        let relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::None);

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
        let relevant_blobs = extract_with_read_modes(&block, ReadMode::Full, ReadMode::None);

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        let Some(boundary) = proofs.batch.completeness_proof.as_mut() else {
            prop_assume!(false);
            return Ok(());
        };

        shift_namespace_proof_range(&mut boundary.last_share_proof, true);

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

    #[test]
    fn proptest_mixed_version_fixture_verification_survives_varied_reads(
        fixture in mixed_version_fixture_strategy(),
        read_mode in read_mode_strategy()
    ) {
        let (block, params) = load_mixed_version_fixture(fixture);
        let relevant_blobs = extract_with_read_modes(&block, read_mode, ReadMode::None);

        prop_assert!(
            !relevant_blobs.batch_blobs.is_empty(),
            "fixture should expose supported v1 batch blobs"
        );
        prop_assert!(relevant_blobs.proof_blobs.is_empty());

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        let skipped_idx = skipped_proof_index(&proofs.batch.inclusion_proof);
        prop_assert!(
            skipped_idx.is_some(),
            "fixture should include skipped v0/tail-padding proofs"
        );

        let result = CelestiaVerifier::new(params)
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "mixed-version verification failed with {:?} for fixture {:?} read_mode {:?}",
            result.err(),
            fixture,
            read_mode
        );
    }

    #[test]
    fn proptest_mutated_skipped_blob_range_returns_err_without_panic(
        fixture in mixed_version_fixture_strategy(),
        read_mode in read_mode_strategy()
    ) {
        let (block, params) = load_mixed_version_fixture(fixture);
        let relevant_blobs = extract_with_read_modes(&block, read_mode, ReadMode::None);
        let mut proofs = get_extraction_proof(&block, &relevant_blobs);

        let Some(skipped_idx) = skipped_proof_index(&proofs.batch.inclusion_proof) else {
            prop_assume!(false);
            return Ok(());
        };

        shift_namespace_proof_range(
            &mut proofs.batch.inclusion_proof[skipped_idx].range_proofs[0].proof,
            true,
        );

        assert_err_without_panic(
            &CelestiaVerifier::new(params),
            &block,
            &relevant_blobs,
            proofs,
            "mutated skipped blob range",
        );
    }

    #[test]
    fn proptest_proof_namespace_fixture_verification_survives_varied_reads(
        fixture in proof_fixture_strategy(),
        batch_mode in read_mode_strategy(),
        proof_mode in read_mode_strategy()
    ) {
        let (block, params) = load_proof_fixture(fixture);
        let relevant_blobs = extract_with_read_modes(&block, batch_mode, proof_mode);

        prop_assert!(
            !relevant_blobs.proof_blobs.is_empty(),
            "fixture should contain proof blobs"
        );

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        assert_subproof_start_indices_align(&block, "batch", &proofs.batch.inclusion_proof);
        assert_subproof_start_indices_align(&block, "proof", &proofs.proof.inclusion_proof);

        let result = CelestiaVerifier::new(params)
            .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "proof-namespace verification failed with {:?} for fixture {:?} batch {:?} proof {:?}",
            result.err(),
            fixture,
            batch_mode,
            proof_mode
        );
    }

    #[test]
    fn proptest_mutated_proof_start_share_idx_returns_err_without_panic(
        fixture in proof_fixture_strategy(),
        batch_mode in read_mode_strategy(),
        proof_mode in read_mode_strategy()
    ) {
        let (block, params) = load_proof_fixture(fixture);
        let relevant_blobs = extract_with_read_modes(&block, batch_mode, proof_mode);
        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        prop_assume!(!proofs.proof.inclusion_proof.is_empty());
        proofs.proof.inclusion_proof[0].range_proofs[0].start_share_idx = proofs.proof
            .inclusion_proof[0]
            .range_proofs[0]
            .start_share_idx
            .saturating_add(1);

        assert_err_without_panic(
            &CelestiaVerifier::new(params),
            &block,
            &relevant_blobs,
            proofs,
            "mutated proof start_share_idx",
        );
    }
}

#[test]
fn protocol_fixture_with_ambiguous_empty_namespace_is_rejected() {
    let (block, params) = synthetic_noncanonical_multirow_absence_fixture();
    let relevant_blobs = RelevantBlobs {
        batch_blobs: Vec::new(),
        proof_blobs: Vec::new(),
    };
    let row_root_count = block
        .header
        .get_row_roots_for_namespace(params.rollup_batch_namespace)
        .count();
    assert!(
        row_root_count > 1,
        "fixture should expose multiple candidate row roots"
    );

    let proofs = get_extraction_proof(&block, &relevant_blobs);
    let err = CelestiaVerifier::new(params)
        .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs)
        .expect_err("ambiguous empty namespace should fail closed");
    assert!(
        err.to_string().contains("MissingBlobs"),
        "expected MissingBlobs, got: {err}"
    );
}
