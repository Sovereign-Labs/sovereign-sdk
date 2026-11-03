pub mod address;
pub mod proofs;

use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::nmt::{Namespace, NamespacedHash, NS_SIZE};
use nmt_rs::NamespacedSha2Hasher;
use sov_rollup_interface::da::{self, DaSpec, RelevantBlobs, RelevantProofs};

use self::address::CelestiaAddress;
use self::proofs::*;
use crate::shares::shares_needed_for_bytes_with_signer;
use crate::types::NamespaceValidationError::{InvalidBlobData, InvalidRowProof};
use crate::types::{
    BlobDataError, BlobWithSender, NamespaceBoundaryProof, NamespaceType, NamespaceValidationError,
    ProofError, RowProofError, TmHash, ValidationError, SUPPORTED_SHARE_VERSION,
};
use crate::CelestiaHeader;

pub use sov_celestia_common::verifier::RollupParams;

#[derive(Clone)]
pub struct CelestiaVerifier {
    rollup_batch_namespace: Namespace,
    rollup_proof_namespace: Namespace,
}

#[derive(
    Default,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    BorshDeserialize,
    BorshSerialize,
    schemars::JsonSchema,
)]
pub struct CelestiaSpec;

impl DaSpec for CelestiaSpec {
    type SlotHash = TmHash;

    type BlockHeader = CelestiaHeader;

    type BlobTransaction = BlobWithSender;

    type TransactionId = TmHash;

    type Address = CelestiaAddress;

    type InclusionMultiProof = Vec<BlobProof>;

    type CompletenessProof = Option<NamespaceBoundaryProof>;

    type ChainParams = RollupParams;
}

impl da::DaVerifier for CelestiaVerifier {
    type Spec = CelestiaSpec;

    type Error = ValidationError;

    fn new(params: <Self::Spec as DaSpec>::ChainParams) -> Self {
        Self {
            rollup_proof_namespace: params.rollup_proof_namespace,
            rollup_batch_namespace: params.rollup_batch_namespace,
        }
    }

    #[cfg_attr(feature = "bench", sov_modules_macros::cycle_tracker)]
    fn verify_relevant_tx_list(
        &self,
        block_header: &<Self::Spec as DaSpec>::BlockHeader,
        relevant_blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
        relevant_proofs: RelevantProofs<
            <Self::Spec as DaSpec>::InclusionMultiProof,
            <Self::Spec as DaSpec>::CompletenessProof,
        >,
    ) -> Result<(), Self::Error> {
        block_header.validate_dah()?;

        // Batch blobs
        Self::verify_blobs(
            block_header,
            &relevant_blobs.batch_blobs,
            self.rollup_batch_namespace,
            relevant_proofs.batch.inclusion_proof.clone(),
            relevant_proofs.batch.completeness_proof.clone(),
        )
        .map_err(|error| ValidationError::NamespaceValidationError {
            namespace: NamespaceType::Batch,
            error,
        })?;

        // Proof blobs
        Self::verify_blobs(
            block_header,
            &relevant_blobs.proof_blobs,
            self.rollup_proof_namespace,
            relevant_proofs.proof.inclusion_proof.clone(),
            relevant_proofs.proof.completeness_proof.clone(),
        )
        .map_err(|error| ValidationError::NamespaceValidationError {
            namespace: NamespaceType::Proof,
            error,
        })?;

        Ok(())
    }
}

impl CelestiaVerifier {
    /// Input:
    /// * reference [`CelestiaHeader`] to verify against. This data is trusted by this point.
    /// * the slice of [`BlobWithSender`] that has been extracted. Not trusted.
    /// * Namespace of the blobs. Trusted parameter of the whole rollup
    /// * Option<NamespaceBoundaryProof> is needed to do the whole namespace completeness check
    ///
    /// This method verifies that:
    /// 1. Each blob provided as an input really is included
    /// 2. No blobs were skipped
    /// 3. The provided blobs really do constitute the entirety of the namespace
    fn verify_blobs(
        block_header: &CelestiaHeader,
        blobs: &[BlobWithSender],
        namespace: Namespace,
        inclusion_proof: Vec<BlobProof>,
        namespace_boundary_proof: Option<NamespaceBoundaryProof>,
    ) -> Result<(), NamespaceValidationError> {
        let namespace_row_roots = block_header
            .dah
            .row_roots()
            .iter()
            .filter(|root| root.contains::<NamespacedSha2Hasher<NS_SIZE>>(namespace.into()))
            .collect::<Vec<_>>();
        sov_celestia_common::verifier::verify_namespace(
            block_header,
            blobs,
            namespace,
            inclusion_proof,
            namespace_boundary_proof,
            supported_share,
            |blob, proof| {
                authenticate_blob_data(block_header, &namespace_row_roots, namespace, blob, proof)
            },
        )
    }
}

pub(crate) fn supported_share(share: &celestia_types::Share) -> bool {
    share
        .info_byte()
        .is_some_and(|info| info.version() == SUPPORTED_SHARE_VERSION)
        && !crate::shares::is_tail_padding(share)
}

/// Authenticates one supported blob against its inclusion proof.
/// 1. Verifies that the passed `BlobProof` is valid.
/// 2. Verifies signer and payload bytes against the data actually read by the rollup.
/// 3. Verifies at least one share and that the number of proven shares matches the bytes read.
///
/// Continuity and first-blob left-boundary checks are performed by `verify_blobs` before this call.
///
/// Returns the total number of shares occupied by this blob.
fn authenticate_blob_data(
    block_header: &CelestiaHeader,
    namespace_row_roots: &[&NamespacedHash],
    namespace: Namespace,
    blob: &BlobWithSender,
    blob_row_proof: BlobProof,
) -> Result<usize, NamespaceValidationError> {
    if namespace == Namespace::PARITY_SHARE {
        // Parity share namespace should not be verified
        return Err(InvalidBlobData(BlobDataError::UnexpectedBlobs));
    }
    // The accumulator length is considered trusted as a record of the bytes that the rollup saw.
    // This does not mean that it can be trusted to contain the correct bytes.
    // Share-occupancy math is defined over DA bytes, hence the compressed accessor.
    let blob_data_read = blob.compressed_verified_data();
    let first_share = blob_row_proof.first_share().map_err(InvalidRowProof)?;
    let has_signer = first_share.signer().is_some();
    let first_share_payload = first_share
        .payload()
        // Parity share namespace should not be verified
        .ok_or(InvalidBlobData(BlobDataError::UnexpectedBlobs))?;
    if crate::envelope::has_magic_prefix(first_share_payload) {
        let required_prefix_len =
            crate::envelope::ENVELOPE_HEADER_LEN.min(blob.compressed_total_len());
        if blob_data_read.len() < required_prefix_len {
            return Err(InvalidBlobData(BlobDataError::NonMatchingShare));
        }
    }
    let num_shares_to_prove =
        shares_needed_for_bytes_with_signer(blob_data_read.len(), has_signer).max(1);
    let num_shares_with_proofs = blob_row_proof
        .range_proofs
        .iter()
        .map(|sub_proof| sub_proof.shares.len())
        .sum::<usize>();

    if num_shares_to_prove != num_shares_with_proofs {
        // Return earlier, so we don't have to deal with verification.
        return Err(InvalidRowProof(RowProofError::WrongNumberOfShares {
            expected: num_shares_to_prove,
            actual: num_shares_with_proofs,
        }));
    }
    let mut shares_proven = 0;
    // This is used to point full blob data to content of particular share for comparison.
    let mut start_blob_data_idx = 0;
    // This is filled later based on the data from share
    let mut sequence_length: Option<u64> = None;
    let mut signer_checked = false;

    // Verifying row by row
    for (range_idx, sub_proof) in blob_row_proof.range_proofs.into_iter().enumerate() {
        let row_number = block_header.calculate_row_number_for_share(sub_proof.start_share_idx);
        let row_root = namespace_row_roots[row_number];

        // Subrange verification
        {
            let RangeProof { shares, proof, .. } = sub_proof;
            for (share_idx, share) in shares.iter().enumerate() {
                // The first share of the first range means starting share:
                // 1. Sequence length
                // 2. Signer.
                if range_idx == 0 && share_idx == 0 {
                    sequence_length = share.sequence_length().map(u64::from);
                    let Some(recovered_signer) = share.signer().map(CelestiaAddress) else {
                        return Err(InvalidBlobData(BlobDataError::NonMatchingShare));
                    };
                    if recovered_signer != blob.sender {
                        return Err(InvalidBlobData(BlobDataError::WrongSender));
                    }
                    signer_checked = true;
                }
                // Verify blob data matching share data.
                {
                    let share_data = share
                        .payload()
                        // Parity share namespace should not be verified
                        .ok_or(InvalidBlobData(BlobDataError::UnexpectedBlobs))?;
                    let (blob_data_end, share_data_end) =
                        if (start_blob_data_idx + share_data.len()) >= blob_data_read.len() {
                            (
                                blob_data_read.len(),
                                blob_data_read.len() - start_blob_data_idx,
                            )
                        } else {
                            (start_blob_data_idx + share_data.len(), share_data.len())
                        };
                    let blob_chunk = &blob_data_read[start_blob_data_idx..blob_data_end];
                    let data_read = &share_data[0..share_data_end];
                    if blob_chunk != data_read {
                        return Err(InvalidBlobData(BlobDataError::NonMatchingShare));
                    }
                    start_blob_data_idx += share_data.len();
                }
            }

            let raw_leaves = shares.iter().map(|s| s.data()).collect::<Vec<_>>();

            proof
                .verify_range(row_root, &raw_leaves, namespace.into())
                .map_err(|e| InvalidRowProof(RowProofError::ProofError(ProofError::Invalid(e))))?;
            shares_proven += raw_leaves.len();
        }
    }

    // Safety checks
    // We shouldn't reach with incorrect number of proven shares by this point.
    // Because the number of shares in each range proof has been compared with the number of shares
    // needed to prove bytes read by blob.
    // This failure means a bug in the preceding code.
    assert_eq!(
        shares_proven, num_shares_to_prove,
        "Bug. Wrong number of shares proven."
    );
    // There should be always at least 1 share per blob,
    // so a signer should always be validated by this point.
    // Failure means a bug.
    debug_assert!(signer_checked, "Bug. Signer checking has been skipped");
    let sequence_length = sequence_length.expect("sequence length should be set by this point");

    // `compressed_total_len()` is backed by the prover-supplied witness (it backs
    // `BlobReaderTrait::total_len()`), which feeds gas accounting, size gates, and the
    // malformed-blob slash check in the STF / sov-blob-storage. The witness is never
    // otherwise checked against the DA shares (only the accumulator *content* is
    // authenticated, as a prefix), so pin it to the `sequence_length` proven from the
    // authenticated first share — otherwise a malicious prover could attest a state
    // transition computed over a forged blob length. Use the compression-aware
    // `compressed_total_len()` accessor so the comparison stays tied to the DA bytes.
    let claimed_total_len = blob.compressed_total_len() as u64;
    if claimed_total_len != sequence_length {
        return Err(InvalidBlobData(BlobDataError::MismatchedBlobLength {
            expected: sequence_length,
            actual: claimed_total_len,
        }));
    }
    let shares_occupied_total =
        shares_needed_for_bytes_with_signer(sequence_length as usize, has_signer);
    Ok(shares_occupied_total)
}

#[cfg(test)]
mod tests {
    use sov_celestia_common::types::NamespaceValidationError::IncompleteNamespace;
    use sov_celestia_common::types::{IncompleteNamespaceError, ProofError};
    use sov_celestia_common::verifier::{
        check_namespace_end_boundary, prevalidate_blobs, PreValidationOutput,
    };

    use super::*;
    use crate::test_helper::files::{
        from_testnet_no_shares, from_testnet_with_tail_padding,
        with_mixed_v0_and_v1_multi_v1_parity_boundary, with_namespace_padding,
        with_parity_boundary_followed_by_namespace, with_rollup_batch_data,
    };
    use crate::types::FilteredCelestiaBlock;

    fn fixture_with_multiple_candidate_row_roots() -> (FilteredCelestiaBlock, Namespace) {
        let candidates = [
            (
                from_testnet_no_shares::filtered_block(),
                from_testnet_no_shares::ROLLUP_PARAMS.rollup_batch_namespace,
            ),
            (
                from_testnet_with_tail_padding::filtered_block(),
                from_testnet_with_tail_padding::ROLLUP_PARAMS.rollup_batch_namespace,
            ),
            (
                with_namespace_padding::filtered_block(),
                with_namespace_padding::ROLLUP_PARAMS.rollup_batch_namespace,
            ),
        ];

        for (block, namespace) in candidates {
            let root_count = block.header.get_row_roots_for_namespace(namespace).count();
            if root_count > 1 {
                return (block, namespace);
            }
        }

        panic!("No fixture with multiple candidate row roots found");
    }

    fn parity_boundary_row_boundary_proof(
        block: &FilteredCelestiaBlock,
        namespace: Namespace,
        fixture_name: &str,
    ) -> (usize, NamespaceBoundaryProof) {
        let row_len = block.header.row_length();
        let rows = block.rollup_batch_data.data.rows();
        let Some((row_idx, row)) = rows
            .iter()
            .enumerate()
            .take(rows.len().saturating_sub(1))
            .find(|(idx, row)| {
                if row.shares.is_empty()
                    || row.proof.end_idx() as usize != row_len
                    || row.shares.last().is_none_or(|share| share.is_parity())
                {
                    return false;
                }
                !rows[idx + 1].shares.is_empty()
            })
        else {
            panic!("{fixture_name} fixture does not contain expected parity-boundary row shape");
        };

        let all_before_last = &row.shares[..row.shares.len().saturating_sub(1)];
        let last_share = row
            .shares
            .last()
            .expect("Boundary row should contain at least one share")
            .clone();
        let last_share_proof = row
            .proof
            .narrow_range(all_before_last, &[], *namespace)
            .expect("Failed to build boundary proof for parity-boundary row");
        let right_sibling = last_share_proof
            .leftmost_right_sibling()
            .expect("Expected right sibling for parity-boundary proof");
        assert_eq!(
            right_sibling.min_namespace(),
            *Namespace::PARITY_SHARE,
            "Fixture should expose parity right sibling at row boundary"
        );

        (
            row_idx,
            NamespaceBoundaryProof {
                last_share_proof: last_share_proof.into(),
                last_share: Some(last_share),
            },
        )
    }

    #[test]
    fn prevalidate_rejects_ambiguous_empty_namespace_without_inclusion_proofs() {
        let (block, namespace) = fixture_with_multiple_candidate_row_roots();
        let namespace_row_roots = block
            .header
            .get_row_roots_for_namespace(namespace)
            .collect::<Vec<_>>();
        assert!(
            namespace_row_roots.len() > 1,
            "Test fixture must have multiple candidate row roots"
        );

        let empty_blobs: Vec<BlobWithSender> = Vec::new();
        let empty_inclusion_proof: Vec<BlobProof> = Vec::new();
        let err = match prevalidate_blobs(
            &namespace_row_roots,
            &empty_blobs,
            namespace,
            &empty_inclusion_proof,
            &None,
        ) {
            Err(err) => err,
            Ok(_) => panic!("Expected MissingBlobs for ambiguous empty namespace"),
        };

        assert!(
            matches!(
                err,
                IncompleteNamespace(IncompleteNamespaceError::MissingBlobs)
            ),
            "Expected MissingBlobs, got: {err:?}"
        );
    }

    #[test]
    fn prevalidate_continues_when_inclusion_proofs_are_present_even_if_no_blobs() {
        let (block, namespace) = fixture_with_multiple_candidate_row_roots();
        let namespace_row_roots = block
            .header
            .get_row_roots_for_namespace(namespace)
            .collect::<Vec<_>>();
        assert!(
            namespace_row_roots.len() > 1,
            "Test fixture must have multiple candidate row roots"
        );

        let empty_blobs: Vec<BlobWithSender> = Vec::new();
        let non_empty_inclusion_proof = vec![BlobProof {
            range_proofs: Vec::new(),
        }];
        let output = prevalidate_blobs(
            &namespace_row_roots,
            &empty_blobs,
            namespace,
            &non_empty_inclusion_proof,
            &None,
        )
        .unwrap();

        assert!(
            matches!(output, PreValidationOutput::ContinueVerification),
            "Expected ContinueVerification, got: {output:?}"
        );
    }

    #[test]
    fn prevalidate_single_row_requires_boundary_proof() {
        let block = with_rollup_batch_data::filtered_block();
        let namespace = with_rollup_batch_data::ROLLUP_PARAMS.rollup_batch_namespace;
        let namespace_row_roots = block
            .header
            .get_row_roots_for_namespace(namespace)
            .collect::<Vec<_>>();
        assert_eq!(
            namespace_row_roots.len(),
            1,
            "Test fixture must have exactly one candidate row root"
        );

        let empty_blobs: Vec<BlobWithSender> = Vec::new();
        let empty_inclusion_proof: Vec<BlobProof> = Vec::new();
        let err = match prevalidate_blobs(
            &namespace_row_roots,
            &empty_blobs,
            namespace,
            &empty_inclusion_proof,
            &None,
        ) {
            Err(err) => err,
            Ok(_) => panic!("Expected missing completeness proof for single-row absence path"),
        };

        assert!(
            matches!(
                err,
                IncompleteNamespace(IncompleteNamespaceError::ProofError(ProofError::Missing))
            ),
            "Expected ProofError::Missing, got: {err:?}"
        );
    }

    #[test]
    fn boundary_proof_for_non_last_row_is_rejected() {
        let block = from_testnet_with_tail_padding::filtered_block();
        let namespace = from_testnet_with_tail_padding::ROLLUP_PARAMS.rollup_batch_namespace;
        let namespace_row_roots = block
            .header
            .get_row_roots_for_namespace(namespace)
            .collect::<Vec<_>>();
        assert!(
            namespace_row_roots.len() > 1,
            "Fixture must have multiple candidate namespace row roots"
        );

        let first_row = &block.rollup_batch_data.data.rows()[0];
        assert!(
            !first_row.shares.is_empty(),
            "First namespace row should contain shares"
        );
        let all_before_last = &first_row.shares[..first_row.shares.len().saturating_sub(1)];
        let last_share = first_row
            .shares
            .last()
            .expect("First row should contain at least one share")
            .clone();
        let last_share_proof = first_row
            .proof
            .narrow_range(all_before_last, &[], *namespace)
            .expect("Failed to build boundary proof for first row");
        let boundary_proof = NamespaceBoundaryProof {
            last_share_proof: last_share_proof.into(),
            last_share: Some(last_share),
        };

        // Force computed row_idx = 0 (first row), which is not the last row.
        let last_proven_share_idx = boundary_proof.last_share_proof.start_idx() as usize;
        let err = check_namespace_end_boundary(
            &block.header,
            &namespace_row_roots,
            namespace,
            last_proven_share_idx,
            Some(boundary_proof),
        )
        .expect_err("Boundary proof from non-last row must be rejected");

        assert!(
            matches!(
                err,
                IncompleteNamespace(IncompleteNamespaceError::MissingBlobs)
            ),
            "Expected MissingBlobs, got: {err:?}"
        );
    }

    /// Builds a parity-boundary proof for the given block/namespace and asserts that
    /// `check_namespace_end_boundary` rejects it with `MissingBlobs`.
    fn assert_parity_boundary_rejected(
        block: &FilteredCelestiaBlock,
        namespace: Namespace,
        namespace_row_roots: &[&NamespacedHash],
        label: &str,
    ) {
        let row_len = block.header.row_length();
        let (row_idx, boundary_proof) = parity_boundary_row_boundary_proof(block, namespace, label);
        let proof_start = boundary_proof.last_share_proof.start_idx() as usize;
        let last_proven_share_idx = row_idx
            .checked_mul(row_len)
            .and_then(|offset| offset.checked_add(proof_start))
            .expect("Share index overflow");
        let err = check_namespace_end_boundary(
            &block.header,
            namespace_row_roots,
            namespace,
            last_proven_share_idx,
            Some(boundary_proof),
        )
        .expect_err(&format!(
            "Boundary proof from non-last {label} namespace row must be rejected"
        ));

        assert!(
            matches!(
                err,
                IncompleteNamespace(IncompleteNamespaceError::MissingBlobs)
            ),
            "Expected MissingBlobs for {label}, got: {err:?}"
        );
    }

    #[test]
    fn parity_boundary_row_with_next_row_namespace_is_rejected_if_used_as_final_boundary() {
        let block = with_parity_boundary_followed_by_namespace::filtered_block();
        let namespace =
            with_parity_boundary_followed_by_namespace::ROLLUP_PARAMS.rollup_batch_namespace;
        let namespace_row_roots = block
            .header
            .get_row_roots_for_namespace(namespace)
            .collect::<Vec<_>>();
        assert!(
            namespace_row_roots.len() > 1,
            "Fixture must have multiple candidate namespace row roots"
        );

        assert_parity_boundary_rejected(&block, namespace, &namespace_row_roots, "parity boundary");
    }

    #[test]
    fn mixed_multi_v1_parity_boundary_row_is_rejected_if_used_as_final_boundary() {
        let block = with_mixed_v0_and_v1_multi_v1_parity_boundary::filtered_block();
        let namespace =
            with_mixed_v0_and_v1_multi_v1_parity_boundary::ROLLUP_PARAMS.rollup_batch_namespace;
        let namespace_row_roots = block
            .header
            .get_row_roots_for_namespace(namespace)
            .collect::<Vec<_>>();
        assert!(
            namespace_row_roots.len() > 1,
            "Mixed multi-v1 fixture must have multiple candidate namespace row roots"
        );

        assert_parity_boundary_rejected(
            &block,
            namespace,
            &namespace_row_roots,
            "mixed multi-v1 parity boundary",
        );
    }
}
