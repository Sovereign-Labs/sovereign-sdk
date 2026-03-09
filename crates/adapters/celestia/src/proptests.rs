use std::panic::{catch_unwind, AssertUnwindSafe};

use celestia_types::state::AddressTrait;
use nmt_rs::nmt_proof::NamespaceProof as NmtNamespaceProof;
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use sov_rollup_interface::da::{BlobReaderTrait, DaVerifier};

use crate::da_service::{extract_relevant_blobs, get_extraction_proof};
use crate::test_support::{
    append_segmented_blobs, assert_subproof_start_indices_align, build_block_from_ods,
    make_blob_shares, rollup_params, signer_for_index, NS_BATCH, PREFIX_NAMESPACES,
    SUFFIX_NAMESPACES,
};
use crate::types::{FilteredCelestiaBlock, NamespaceBoundaryProof};
use crate::verifier::proofs::BlobProof;
use crate::verifier::CelestiaVerifier;

#[derive(Debug, Clone)]
pub(crate) struct MultiRowBatchCase {
    pub(crate) ods_width: usize,
    pub(crate) prefix_rows: usize,
    pub(crate) batch_rows_full: usize,
    pub(crate) suffix_rows: usize,
    pub(crate) batch_blob_shares: Vec<usize>,
    pub(crate) prefix_segments: Vec<usize>,
    pub(crate) suffix_segments: Vec<usize>,
    pub(crate) seed: u8,
}

impl MultiRowBatchCase {
    pub(crate) fn prefix_total(&self) -> usize {
        self.prefix_rows * self.ods_width
    }

    pub(crate) fn batch_total(&self) -> usize {
        self.batch_rows_full * self.ods_width
    }

    pub(crate) fn suffix_total(&self) -> usize {
        self.suffix_rows * self.ods_width
    }

    pub(crate) fn build_block(&self) -> FilteredCelestiaBlock {
        build_block_from_ods(self.build_ods_shares())
    }

    pub(crate) fn build_ods_shares(&self) -> Vec<Vec<u8>> {
        let prefix_total = self.prefix_total();
        let batch_total = self.batch_total();
        let suffix_total = self.suffix_total();
        let total_shares = self.ods_width * self.ods_width;

        assert_eq!(prefix_total, self.prefix_segments.iter().sum::<usize>());
        assert_eq!(batch_total, self.batch_blob_shares.iter().sum::<usize>());
        assert_eq!(suffix_total, self.suffix_segments.iter().sum::<usize>());
        assert_eq!(prefix_total + batch_total + suffix_total, total_shares);

        let mut shares = Vec::with_capacity(total_shares);
        append_segmented_blobs(
            &mut shares,
            &self.prefix_segments,
            &PREFIX_NAMESPACES,
            None,
            self.seed,
        );
        for (idx, share_count) in self.batch_blob_shares.iter().enumerate() {
            shares.extend(make_blob_shares(
                NS_BATCH,
                *share_count,
                Some(signer_for_index(self.seed, idx)),
                self.seed.wrapping_add(idx as u8),
            ));
        }
        append_segmented_blobs(
            &mut shares,
            &self.suffix_segments,
            &SUFFIX_NAMESPACES,
            None,
            self.seed.wrapping_add(0x80),
        );
        assert_eq!(shares.len(), total_shares);

        shares
    }
}

fn multi_row_case_strategy() -> BoxedStrategy<MultiRowBatchCase> {
    prop_oneof![Just(4usize), Just(8usize)]
        .prop_flat_map(|ods_width| {
            (1..ods_width).prop_flat_map(move |prefix_rows| {
                let remaining = ods_width - prefix_rows - 1;
                (0..=remaining, any::<u8>()).prop_map(move |(suffix_rows, seed)| {
                    let batch_rows = ods_width - prefix_rows - suffix_rows;
                    let prefix_total = prefix_rows * ods_width;
                    let batch_total = batch_rows * ods_width;
                    let suffix_total = suffix_rows * ods_width;

                    MultiRowBatchCase {
                        ods_width,
                        prefix_rows,
                        batch_rows_full: batch_rows,
                        suffix_rows,
                        batch_blob_shares: vec![batch_total],
                        prefix_segments: vec![prefix_total],
                        suffix_segments: if suffix_total == 0 {
                            Vec::new()
                        } else {
                            vec![suffix_total]
                        },
                        seed,
                    }
                })
            })
        })
        .boxed()
}

#[derive(Debug, Clone)]
pub(crate) struct MidRowBatchCase {
    pub(crate) ods_width: usize,
    pub(crate) prefix_rows: usize,
    pub(crate) prefix_len: usize,
    pub(crate) batch_rows_full: usize,
    pub(crate) last_batch_len: usize,
    pub(crate) suffix_rows: usize,
    pub(crate) seed: u8,
}

impl MidRowBatchCase {
    pub(crate) fn prefix_total(&self) -> usize {
        self.prefix_rows * self.ods_width + self.prefix_len
    }

    pub(crate) fn batch_total(&self) -> usize {
        (self.ods_width - self.prefix_len)
            + self.batch_rows_full * self.ods_width
            + self.last_batch_len
    }

    pub(crate) fn suffix_total(&self) -> usize {
        self.suffix_rows * self.ods_width + (self.ods_width - self.last_batch_len)
    }

    pub(crate) fn build_block(&self) -> FilteredCelestiaBlock {
        let prefix_total = self.prefix_total();
        let batch_total = self.batch_total();
        let suffix_total = self.suffix_total();
        let total_shares = self.ods_width * self.ods_width;

        assert_eq!(prefix_total + batch_total + suffix_total, total_shares);

        let mut shares = Vec::with_capacity(total_shares);
        append_segmented_blobs(
            &mut shares,
            &[prefix_total],
            &PREFIX_NAMESPACES,
            None,
            self.seed,
        );
        shares.extend(make_blob_shares(
            NS_BATCH,
            batch_total,
            Some(signer_for_index(self.seed, 0)),
            self.seed,
        ));
        append_segmented_blobs(
            &mut shares,
            &[suffix_total],
            &SUFFIX_NAMESPACES,
            None,
            self.seed.wrapping_add(0x80),
        );
        assert_eq!(shares.len(), total_shares);

        build_block_from_ods(shares)
    }
}

fn mid_row_case_strategy() -> BoxedStrategy<MidRowBatchCase> {
    prop_oneof![Just(4usize), Just(8usize)]
        .prop_flat_map(|ods_width| {
            let max_batch_rows_full = ods_width - 2;
            (0..=max_batch_rows_full).prop_flat_map(move |batch_rows_full| {
                let max_prefix_rows = ods_width - 2 - batch_rows_full;
                (0..=max_prefix_rows, 1..ods_width, 1..ods_width, any::<u8>()).prop_map(
                    move |(prefix_rows, prefix_len, last_batch_len, seed)| MidRowBatchCase {
                        ods_width,
                        prefix_rows,
                        prefix_len,
                        batch_rows_full,
                        last_batch_len,
                        suffix_rows: ods_width - 2 - prefix_rows - batch_rows_full,
                        seed,
                    },
                )
            })
        })
        .boxed()
}

#[derive(Debug, Clone)]
struct SingleRowMidRowCase {
    ods_width: usize,
    prefix_rows: usize,
    prefix_len: usize,
    batch_len: usize,
    suffix_rows: usize,
    seed: u8,
}

impl SingleRowMidRowCase {
    fn build_block(&self) -> FilteredCelestiaBlock {
        let suffix_len = self
            .ods_width
            .checked_sub(self.prefix_len)
            .and_then(|remaining| remaining.checked_sub(self.batch_len))
            .expect("invalid case: prefix_len + batch_len must be < ods_width");
        let prefix_total = self.prefix_rows * self.ods_width + self.prefix_len;
        let suffix_total = self.suffix_rows * self.ods_width + suffix_len;
        let total_shares = self.ods_width * self.ods_width;

        assert!(
            suffix_len > 0,
            "single-row mid-row case requires suffix in row"
        );
        assert_eq!(prefix_total + self.batch_len + suffix_total, total_shares);

        let mut shares = Vec::with_capacity(total_shares);
        if prefix_total > 0 {
            append_segmented_blobs(
                &mut shares,
                &[prefix_total],
                &PREFIX_NAMESPACES,
                None,
                self.seed,
            );
        }
        shares.extend(make_blob_shares(
            NS_BATCH,
            self.batch_len,
            Some(signer_for_index(self.seed, 0)),
            self.seed,
        ));
        if suffix_total > 0 {
            append_segmented_blobs(
                &mut shares,
                &[suffix_total],
                &SUFFIX_NAMESPACES,
                None,
                self.seed.wrapping_add(0x80),
            );
        }
        assert_eq!(shares.len(), total_shares);

        build_block_from_ods(shares)
    }
}

fn single_row_mid_row_case_strategy() -> BoxedStrategy<SingleRowMidRowCase> {
    prop_oneof![Just(4usize), Just(8usize)]
        .prop_flat_map(|ods_width| {
            (0..ods_width, 1..(ods_width - 2), any::<u8>()).prop_flat_map(
                move |(prefix_rows, prefix_len, seed)| {
                    let suffix_rows = ods_width - prefix_rows - 1;
                    (2..(ods_width - prefix_len)).prop_map(move |batch_len| SingleRowMidRowCase {
                        ods_width,
                        prefix_rows,
                        prefix_len,
                        batch_len,
                        suffix_rows,
                        seed,
                    })
                },
            )
        })
        .boxed()
}

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
        let verifier = CelestiaVerifier::new(rollup_params());
        relevant_blobs.batch_blobs.clear();

        prop_assert!(verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, proofs).is_err());
    }

    #[test]
    #[ignore = "BUG SPEC: mid-row completeness proof rejects valid layouts (see docs/celestia/multirow-absence-redesign-prompt.md)"]
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
    #[ignore = "BUG SPEC: single-row mid-row completeness proof rejects valid layouts (see docs/celestia/multirow-absence-redesign-prompt.md)"]
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
}

#[test]
#[ignore = "BUG SPEC: mid-row completeness proof rejects valid layouts (see docs/celestia/multirow-absence-redesign-prompt.md)"]
fn test_mid_row_full_verification_manual() {
    let case = MidRowBatchCase {
        ods_width: 8,
        prefix_rows: 0,
        prefix_len: 3,
        batch_rows_full: 5,
        last_batch_len: 6,
        suffix_rows: 1,
        seed: 123,
    };
    let block = case.build_block();
    let mut relevant_blobs = extract_relevant_blobs(&block);

    assert_eq!(relevant_blobs.batch_blobs.len(), 1);
    for blob in &mut relevant_blobs.batch_blobs {
        blob.advance(blob.total_len());
    }

    let proofs = get_extraction_proof(&block, &relevant_blobs);
    assert_subproof_start_indices_align(&block, "batch", &proofs.batch.inclusion_proof);
    assert_subproof_start_indices_align(&block, "proof", &proofs.proof.inclusion_proof);

    assert!(CelestiaVerifier::new(rollup_params())
        .verify_relevant_tx_list(&block.header, &relevant_blobs, proofs)
        .is_ok());
}
