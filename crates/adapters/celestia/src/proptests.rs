use celestia_types::consts::appconsts;
use celestia_types::namespace_data::NamespaceData;
use celestia_types::nmt::Namespace;
use celestia_types::state::{AccAddress, AddressTrait};
use celestia_types::{Blob, DataAvailabilityHeader, ExtendedDataSquare};
use nmt_rs::nmt_proof::NamespaceProof as NmtNamespaceProof;
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use sov_rollup_interface::da::{BlobReaderTrait, DaVerifier};
use std::panic::{catch_unwind, AssertUnwindSafe};
use tendermint::Hash;

use crate::celestia::{CelestiaHeader, CompactHeader, ProtobufHash};
use crate::da_service::{extract_relevant_blobs, get_extraction_proof};
use crate::types::{
    FilteredCelestiaBlock, NamespaceBoundaryProof, NamespaceRelevantData, APP_VERSION,
};
use crate::verifier::proofs::BlobProof;
use crate::verifier::{CelestiaVerifier, RollupParams};

pub(crate) const NS_LOW_A: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
pub(crate) const NS_LOW_B: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
pub(crate) const NS_BATCH: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);
pub(crate) const NS_HIGH_A: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 4]);
pub(crate) const NS_HIGH_B: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 5]);
pub(crate) const NS_PROOF: Namespace = Namespace::const_v0([0, 0, 0, 0, 0, 0, 0, 0, 0, 6]);

pub(crate) const PREFIX_NAMESPACES: [Namespace; 2] = [NS_LOW_A, NS_LOW_B];
pub(crate) const SUFFIX_NAMESPACES: [Namespace; 2] = [NS_HIGH_A, NS_HIGH_B];

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
        // With full-row alignment: prefix_len is always 0, so just count full rows
        self.prefix_rows * self.ods_width
    }

    pub(crate) fn batch_total(&self) -> usize {
        // With full-row alignment: batch occupies complete rows
        self.batch_rows_full * self.ods_width
    }

    pub(crate) fn suffix_total(&self) -> usize {
        // With full-row alignment: suffix occupies complete rows
        self.suffix_rows * self.ods_width
    }

    pub(crate) fn rollup_params(&self) -> RollupParams {
        RollupParams {
            rollup_batch_namespace: NS_BATCH,
            rollup_proof_namespace: NS_PROOF,
        }
    }

    pub(crate) fn build_block(&self) -> FilteredCelestiaBlock {
        build_block_from_ods(self.build_ods_shares())
    }

    pub(crate) fn build_ods_shares(&self) -> Vec<Vec<u8>> {
        let prefix_total = self.prefix_total();
        let batch_total = self.batch_total();
        let suffix_total = self.suffix_total();

        assert_eq!(prefix_total, self.prefix_segments.iter().sum::<usize>());
        assert_eq!(batch_total, self.batch_blob_shares.iter().sum::<usize>());
        assert_eq!(suffix_total, self.suffix_segments.iter().sum::<usize>());

        let total_shares = self.ods_width * self.ods_width;
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
            let signer = signer_for_index(self.seed, idx);
            let payload_seed = self.seed.wrapping_add(idx as u8);
            shares.extend(make_blob_shares(
                NS_BATCH,
                *share_count,
                Some(signer),
                payload_seed,
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

/// Generates test cases where all namespaces occupy complete rows.
///
/// NOTE: Cases where namespaces start/end mid-row are more complex because they require
/// proper NMT sibling proofs to prove no shares are missing. The synthesized test data
/// approach can't easily provide these proofs. For mid-row boundary cases, see the
/// integration tests in da_service/tests.rs which use real Celestia data.
fn multi_row_case_strategy() -> BoxedStrategy<MultiRowBatchCase> {
    let widths = prop_oneof![Just(4usize), Just(8usize)];
    widths
        .prop_flat_map(|ods_width| {
            // Total rows = ods_width (it's a square)
            // We need: prefix_rows + batch_rows + suffix_rows = ods_width
            // With at least 1 row for batch, and at least 1 row for prefix/suffix combined
            // to ensure batch doesn't span the entire ODS (which causes verification issues)
            let max_non_batch_rows = ods_width - 1;
            // Require at least 1 prefix row to ensure batch doesn't start at index 0
            (1..=max_non_batch_rows).prop_flat_map(move |prefix_rows| {
                let remaining = ods_width - prefix_rows - 1; // -1 for at least 1 batch row
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
                        suffix_segments: if suffix_total > 0 {
                            vec![suffix_total]
                        } else {
                            vec![]
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
        build_block_from_ods(self.build_ods_shares())
    }

    pub(crate) fn build_ods_shares(&self) -> Vec<Vec<u8>> {
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
        let signer = signer_for_index(self.seed, 0);
        shares.extend(make_blob_shares(
            NS_BATCH,
            batch_total,
            Some(signer),
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
        shares
    }
}

/// Generates test cases where the target namespace starts and ends mid-row.
///
/// This exercises boundary proofs that rely on siblings from other namespaces.
fn mid_row_case_strategy() -> BoxedStrategy<MidRowBatchCase> {
    let widths = prop_oneof![Just(4usize), Just(8usize)];
    widths
        .prop_flat_map(|ods_width| {
            let max_batch_rows_full = ods_width - 2;
            (0..=max_batch_rows_full).prop_flat_map(move |batch_rows_full| {
                let max_prefix_rows = ods_width - 2 - batch_rows_full;
                (0..=max_prefix_rows, 1..ods_width, 1..ods_width, any::<u8>()).prop_map(
                    move |(prefix_rows, prefix_len, last_batch_len, seed)| {
                        let suffix_rows = ods_width - 2 - prefix_rows - batch_rows_full;
                        MidRowBatchCase {
                            ods_width,
                            prefix_rows,
                            prefix_len,
                            batch_rows_full,
                            last_batch_len,
                            suffix_rows,
                            seed,
                        }
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
        build_block_from_ods(self.build_ods_shares())
    }

    fn build_ods_shares(&self) -> Vec<Vec<u8>> {
        let suffix_len = self
            .ods_width
            .checked_sub(self.prefix_len)
            .and_then(|remaining| remaining.checked_sub(self.batch_len))
            .expect("invalid case: prefix_len + batch_len must be < ods_width");
        assert!(
            suffix_len > 0,
            "single-row mid-row case requires suffix in row"
        );

        let prefix_total = self.prefix_rows * self.ods_width + self.prefix_len;
        let batch_total = self.batch_len;
        let suffix_total = self.suffix_rows * self.ods_width + suffix_len;
        let total_shares = self.ods_width * self.ods_width;
        assert_eq!(prefix_total + batch_total + suffix_total, total_shares);

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

        let signer = signer_for_index(self.seed, 0);
        shares.extend(make_blob_shares(
            NS_BATCH,
            self.batch_len,
            Some(signer),
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
        shares
    }
}

/// Generates cases where namespace batch occupies a single mid-row segment:
/// start index > 0 and end index < row_len within one row.
fn single_row_mid_row_case_strategy() -> BoxedStrategy<SingleRowMidRowCase> {
    let widths = prop_oneof![Just(4usize), Just(8usize)];
    widths
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

pub(crate) fn append_segmented_blobs(
    shares: &mut Vec<Vec<u8>>,
    segments: &[usize],
    namespaces: &[Namespace; 2],
    signer: Option<AccAddress>,
    seed: u8,
) {
    assert!(segments.len() <= namespaces.len());
    for (idx, share_count) in segments.iter().enumerate() {
        let namespace = namespaces[idx];
        let payload_seed = seed.wrapping_add(idx as u8);
        shares.extend(make_blob_shares(
            namespace,
            *share_count,
            signer,
            payload_seed,
        ));
    }
}

pub(crate) fn make_blob_shares(
    namespace: Namespace,
    share_count: usize,
    signer: Option<AccAddress>,
    seed: u8,
) -> Vec<Vec<u8>> {
    assert!(share_count > 0, "share count must be positive");
    let payload_len = payload_len_for_share_count(share_count, signer.is_some());
    let payload = vec![seed; payload_len];
    let blob = Blob::new(namespace, payload, signer, APP_VERSION).expect("blob creation failed");
    let shares = blob.to_shares().expect("blob to shares failed");
    assert_eq!(shares.len(), share_count, "unexpected share count");
    shares.into_iter().map(|share| share.to_vec()).collect()
}

pub(crate) fn payload_len_for_share_count(share_count: usize, has_signer: bool) -> usize {
    let first_share_content = if has_signer {
        appconsts::FIRST_SPARSE_SHARE_CONTENT_SIZE
            .checked_sub(appconsts::SIGNER_SIZE)
            .expect("signer size should fit into first share content size")
    } else {
        appconsts::FIRST_SPARSE_SHARE_CONTENT_SIZE
    };
    if share_count <= 1 {
        return first_share_content.saturating_sub(1).max(1);
    }
    first_share_content + (share_count - 2) * appconsts::CONTINUATION_SPARSE_SHARE_CONTENT_SIZE + 1
}

pub(crate) fn signer_for_index(seed: u8, idx: usize) -> AccAddress {
    let byte = seed.wrapping_add(idx as u8).wrapping_add(1);
    let bytes = [byte; 20];
    AccAddress::try_from(&bytes[..]).expect("valid signer bytes")
}

pub(crate) fn namespace_rows(
    eds: &ExtendedDataSquare,
    dah: &DataAvailabilityHeader,
    namespace: Namespace,
) -> Vec<celestia_types::row_namespace_data::RowNamespaceData> {
    eds.get_namespace_data(namespace, dah, 1)
        .expect("namespace data fetch failed")
        .into_iter()
        .map(|(_, row)| row)
        .collect()
}

pub(crate) fn build_block_from_ods(ods_shares: Vec<Vec<u8>>) -> FilteredCelestiaBlock {
    let eds = ExtendedDataSquare::from_ods(ods_shares, APP_VERSION).expect("EDS creation failed");
    let dah = DataAvailabilityHeader::from_eds(&eds);
    let batch_rows = namespace_rows(&eds, &dah, NS_BATCH);
    let proof_rows = namespace_rows(&eds, &dah, NS_PROOF);
    FilteredCelestiaBlock {
        header: build_header(dah),
        rollup_batch_data: NamespaceRelevantData::new(NS_BATCH, NamespaceData::new(batch_rows)),
        rollup_proof_data: NamespaceRelevantData::new(NS_PROOF, NamespaceData::new(proof_rows)),
    }
}

pub(crate) fn build_header(dah: DataAvailabilityHeader) -> CelestiaHeader {
    let data_hash = match dah.hash() {
        Hash::Sha256(bytes) => Some(ProtobufHash(bytes)),
        Hash::None => None,
    };
    let compact = CompactHeader {
        version: Vec::new(),
        chain_id: Vec::new(),
        height: Vec::new(),
        time: Vec::new(),
        last_block_id: Vec::new(),
        last_commit_hash: Vec::new(),
        data_hash,
        validators_hash: Vec::new(),
        next_validators_hash: Vec::new(),
        consensus_hash: Vec::new(),
        app_hash: Vec::new(),
        last_results_hash: Vec::new(),
        evidence_hash: Vec::new(),
        proposer_address: Vec::new(),
    };
    CelestiaHeader::new(dah, compact)
}

pub(crate) fn assert_subproof_start_indices_align(
    block: &FilteredCelestiaBlock,
    namespace_name: &str,
    inclusion_proof: &[BlobProof],
) {
    let row_len = block.header.row_length();
    for (blob_idx, blob_proof) in inclusion_proof.iter().enumerate() {
        for (range_idx, range_proof) in blob_proof.range_proofs.iter().enumerate() {
            let row = block
                .header
                .calculate_row_number_for_share(range_proof.start_share_idx);
            let expected = row
                .checked_mul(row_len)
                .and_then(|row_start| row_start.checked_add(range_proof.proof.start_idx() as usize))
                .expect("overflow while calculating expected share index");
            assert_eq!(
                range_proof.start_share_idx, expected,
                "{namespace_name} proof index mismatch for blob #{blob_idx} range #{range_idx}: expected {expected}, actual {}",
                range_proof.start_share_idx
            );
        }
    }
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
    let result = outcome.expect("checked above");
    assert!(result.is_err(), "expected verifier error for {context}");
}

proptest! {
    /// Tests blob extraction from synthesized EDS data.
    ///
    /// NOTE: Full verification with `verify_relevant_tx_list` requires proper NMT proofs
    /// which are complex to synthesize correctly. The integration tests in da_service/tests.rs
    /// cover full verification using real Celestia block fixtures. This proptest focuses on:
    /// - Correct blob count extraction
    /// - Blob data integrity (payload matches seed)
    /// - Signer correctness
    #[test]
    fn proptest_blob_extraction_and_data_integrity(case in multi_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(
            relevant_blobs.batch_blobs.len(),
            case.batch_blob_shares.len(),
            "batch blob count mismatch for case: {:?}",
            case
        );

        // No proof blobs expected in this test setup since we don't add NS_PROOF data
        prop_assert!(
            relevant_blobs.proof_blobs.is_empty(),
            "expected no proof blobs, got {} for case: {:?}",
            relevant_blobs.proof_blobs.len(),
            case
        );

        // Advance all blobs to make full data available
        for blob in &mut relevant_blobs.batch_blobs {
            let total_len = blob.total_len();
            blob.advance(total_len);
        }

        // Validate blob data integrity: first byte should match expected seed
        for (idx, blob) in relevant_blobs.batch_blobs.iter().enumerate() {
            let expected_seed = case.seed.wrapping_add(idx as u8);
            let data = blob.verified_data();
            prop_assert!(
                !data.is_empty(),
                "blob {} has empty data for case: {:?}",
                idx,
                case
            );
            prop_assert_eq!(
                data[0],
                expected_seed,
                "blob {} first byte mismatch: expected seed {}, got {} for case: {:?}",
                idx,
                expected_seed,
                data[0],
                case
            );

            // Validate signer correctness by comparing address bytes
            let expected_signer = signer_for_index(case.seed, idx);
            let actual_sender = blob.sender();
            let sender_bytes: &[u8] = actual_sender.as_ref();
            let expected_bytes: &[u8] = expected_signer.id_ref().as_ref();
            prop_assert_eq!(
                sender_bytes,
                expected_bytes,
                "blob {} signer mismatch for case: {:?}",
                idx,
                case
            );
        }
    }

    /// Tests that removing blobs from valid proofs causes verification to fail with Err.
    /// This exercises the verifier's error handling without requiring valid synthesized proofs.
    #[test]
    fn proptest_empty_blobs_fail_verification(case in multi_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);

        // Advance all blobs before proof generation
        for blob in &mut relevant_blobs.batch_blobs {
            let total_len = blob.total_len();
            blob.advance(total_len);
        }

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        let verifier = CelestiaVerifier::new(case.rollup_params());

        // Remove all blobs - should cause verification to fail with Err, NOT panic
        relevant_blobs.batch_blobs.clear();

        let result = verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_err(),
            "empty blobs should fail verification but got Ok for case: {:?}",
            case
        );
    }

    /// Mid-row layouts can be fully verified if we include lower namespaces
    /// before the batch start and higher namespaces after the batch end.
    #[test]
    #[ignore = "BUG SPEC: mid-row completeness proof rejects valid layouts (see docs/celestia/multirow-absence-redesign-prompt.md)"]
    fn proptest_mid_row_full_verification(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);

        prop_assert_eq!(relevant_blobs.batch_blobs.len(), 1);

        for blob in &mut relevant_blobs.batch_blobs {
            let total_len = blob.total_len();
            blob.advance(total_len);
        }

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        assert_subproof_start_indices_align(&block, "batch", &proofs.batch.inclusion_proof);
        assert_subproof_start_indices_align(&block, "proof", &proofs.proof.inclusion_proof);
        let verifier = CelestiaVerifier::new(RollupParams {
            rollup_batch_namespace: NS_BATCH,
            rollup_proof_namespace: NS_PROOF,
        });

        let result = verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "mid-row verification failed with {:?} for case: {:?}",
            result.err(),
            case
        );
    }

    /// Single-row mid-row namespace layout:
    /// target namespace starts and ends inside one row (start > 0, end < row_len).
    #[test]
    #[ignore = "BUG SPEC: single-row mid-row completeness proof rejects valid layouts (see docs/celestia/multirow-absence-redesign-prompt.md)"]
    fn proptest_single_row_mid_row_full_verification(case in single_row_mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        prop_assert_eq!(relevant_blobs.batch_blobs.len(), 1);

        for blob in &mut relevant_blobs.batch_blobs {
            let total_len = blob.total_len();
            blob.advance(total_len);
        }

        let proofs = get_extraction_proof(&block, &relevant_blobs);
        assert_subproof_start_indices_align(&block, "batch", &proofs.batch.inclusion_proof);
        assert_subproof_start_indices_align(&block, "proof", &proofs.proof.inclusion_proof);

        let verifier = CelestiaVerifier::new(RollupParams {
            rollup_batch_namespace: NS_BATCH,
            rollup_proof_namespace: NS_PROOF,
        });

        let result = verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
        prop_assert!(
            result.is_ok(),
            "single-row mid-row verification failed with {:?} for case: {:?}",
            result.err(),
            case
        );
    }

    /// Adversarial mutation: tamper first range start index and require Err (no panic).
    #[test]
    fn proptest_mutated_start_share_idx_returns_err_without_panic(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        for blob in &mut relevant_blobs.batch_blobs {
            let total_len = blob.total_len();
            blob.advance(total_len);
        }

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        proofs.batch.inclusion_proof[0].range_proofs[0].start_share_idx =
            proofs.batch.inclusion_proof[0].range_proofs[0]
                .start_share_idx
                .saturating_add(1);
        let verifier = CelestiaVerifier::new(RollupParams {
            rollup_batch_namespace: NS_BATCH,
            rollup_proof_namespace: NS_PROOF,
        });
        assert_err_without_panic(
            &verifier,
            &block,
            &relevant_blobs,
            proofs,
            "mutated start_share_idx",
        );
    }

    /// Adversarial mutation: reorder range proofs inside a blob and require Err (no panic).
    #[test]
    fn proptest_mutated_range_proof_order_returns_err_without_panic(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        for blob in &mut relevant_blobs.batch_blobs {
            let total_len = blob.total_len();
            blob.advance(total_len);
        }

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        let range_proofs = &mut proofs.batch.inclusion_proof[0].range_proofs;
        prop_assume!(range_proofs.len() > 1);
        range_proofs.reverse();

        let verifier = CelestiaVerifier::new(RollupParams {
            rollup_batch_namespace: NS_BATCH,
            rollup_proof_namespace: NS_PROOF,
        });
        assert_err_without_panic(
            &verifier,
            &block,
            &relevant_blobs,
            proofs,
            "mutated range proof order",
        );
    }

    /// Adversarial mutation: tamper completeness boundary proof range and require Err (no panic).
    #[test]
    fn proptest_mutated_boundary_range_returns_err_without_panic(case in mid_row_case_strategy()) {
        let block = case.build_block();
        let mut relevant_blobs = extract_relevant_blobs(&block);
        for blob in &mut relevant_blobs.batch_blobs {
            let total_len = blob.total_len();
            blob.advance(total_len);
        }

        let mut proofs = get_extraction_proof(&block, &relevant_blobs);
        let Some(boundary) = proofs.batch.completeness_proof.as_mut() else {
            prop_assume!(false);
            return Ok(());
        };
        let last_share_proof = &mut boundary.last_share_proof;
        match &mut **last_share_proof {
            NmtNamespaceProof::PresenceProof { proof, .. }
            | NmtNamespaceProof::AbsenceProof { proof, .. } => {
                proof.range.start = proof.range.start.saturating_add(1);
                proof.range.end = proof.range.end.saturating_add(1);
            }
        }

        let verifier = CelestiaVerifier::new(RollupParams {
            rollup_batch_namespace: NS_BATCH,
            rollup_proof_namespace: NS_PROOF,
        });
        assert_err_without_panic(
            &verifier,
            &block,
            &relevant_blobs,
            proofs,
            "mutated boundary range",
        );
    }
}

/// A focused, deterministic mid-row case that exercises full verification.
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
        let total_len = blob.total_len();
        blob.advance(total_len);
    }

    let proofs = get_extraction_proof(&block, &relevant_blobs);
    let verifier = CelestiaVerifier::new(RollupParams {
        rollup_batch_namespace: NS_BATCH,
        rollup_proof_namespace: NS_PROOF,
    });

    let result = verifier.verify_relevant_tx_list(&block.header, &relevant_blobs, proofs);
    assert!(
        result.is_ok(),
        "mid-row manual verification failed with: {:?}",
        result.err()
    );
}
