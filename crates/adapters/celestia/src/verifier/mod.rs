pub mod address;
pub mod proofs;

use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::nmt::{Namespace, NamespacedHash, NS_SIZE};
use nmt_rs::NamespacedSha2Hasher;
use sov_rollup_interface::da::{self, DaSpec, RelevantBlobs, RelevantProofs};

use self::address::CelestiaAddress;
use self::proofs::*;
use crate::shares::shares_needed_for_bytes_with_signer;
use crate::types::NamespaceValidationError::{
    IncompleteNamespace, InvalidBlobData, InvalidRowProof,
};
use crate::types::{
    BlobDataError, BlobWithSender, IncompleteNamespaceError, NamespaceBoundaryProof, NamespaceType,
    NamespaceValidationError, ProofError, RowProofError, TmHash, ValidationError,
    SUPPORTED_SHARE_VERSION,
};
use crate::CelestiaHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RollupParams {
    pub rollup_batch_namespace: Namespace,
    pub rollup_proof_namespace: Namespace,
}

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
        // Proof for the last share, if needed.
        namespace_boundary_proof: Option<NamespaceBoundaryProof>,
    ) -> Result<(), NamespaceValidationError> {
        // This is trusted because it was verified before by block_header.validate_dah()?;
        let namespace_row_roots = block_header
            .dah
            .row_roots()
            .iter()
            .filter(|r| r.contains::<NamespacedSha2Hasher<NS_SIZE>>(namespace.into()))
            .collect::<Vec<_>>();
        if prevalidate_blobs(
            &namespace_row_roots,
            blobs,
            namespace,
            &inclusion_proof,
            &namespace_boundary_proof,
        )?
        .is_early_return()
        {
            return Ok(());
        }

        // This value will be derived from all the shares touched, so it is trusted.
        // This includes shares that have been proven *AND* shares that do not need to be proven
        // because they were not read by the rollup.
        let mut last_validated_share_idx: Option<usize> = None;

        let mut blob_proofs_iter = inclusion_proof.into_iter().peekable();
        let mut blobs_iter = blobs.iter().peekable();
        let mut blob_idx = 0;

        while blobs_iter.peek().is_some() || blob_proofs_iter.peek().is_some() {
            let Some(blob_row_proof) = blob_proofs_iter.next() else {
                return Err(InvalidRowProof(RowProofError::missing_proof()));
            };

            // **Continuity check**
            // Namespace/Row relative index of the very first share.
            // Blob can span across several rows, and in this case there will be as many range proofs as number of rows blob covers.
            // Note: blob should have at least one range proof: proof of the first share.
            // Note: these checks does not verify proof yet, only that it fits into blobs previously seen.
            let blob_proof_range_start = {
                // **Beginning of namespace handling**
                let blob_proof_range_start = if blob_idx == 0 {
                    blob_row_proof.verify_left_boundary(block_header, namespace)?
                } else {
                    // Checking continuity with the previous blob
                    let last_from_previous_blob = last_validated_share_idx
                        .expect("Bug: caller didn't set last_validated_share_idx");
                    blob_row_proof.verify_continuity(last_from_previous_blob)?
                };
                // Continuity inside this blob proofs
                blob_row_proof
                    .enforce_continuity()
                    .map_err(InvalidRowProof)?;
                blob_proof_range_start
            };

            let shares_checked = if blob_row_proof.is_supported_blob()? {
                let Some(blob) = blobs_iter.next() else {
                    return Err(InvalidBlobData(BlobDataError::MoreProofsThanBlobs));
                };
                authenticate_blob_data(
                    block_header,
                    &namespace_row_roots,
                    namespace,
                    blob,
                    blob_row_proof,
                )?
            } else {
                // Non-supported blobs are skipped. But still checked its inclusion.
                verify_skipped_blob(
                    block_header,
                    &namespace_row_roots,
                    namespace,
                    blob_row_proof,
                )?
            };
            // Security note: this index is verifier-derived from accepted continuity checks
            // plus inclusion-verified share occupancy (`shares_checked`), not a direct
            // prover-trusted coordinate.
            last_validated_share_idx = Some(match last_validated_share_idx {
                // blob_proof_range_start is used to keep indexes aligned full row, not just relative to namespace.
                None => blob_proof_range_start + shares_checked - 1,
                Some(prev_idx) => prev_idx + shares_checked,
            });

            blob_idx += 1;
        }

        assert!(
            blobs_iter.peek().is_none(),
            "Blobs iterator should be exhausted"
        );
        assert!(
            blob_proofs_iter.peek().is_none(),
            "Blob proofs iterator should be exhausted"
        );

        let last_proven_share_idx =
            last_validated_share_idx.expect("At least 1 share should be proven by this point");

        check_namespace_end_boundary(
            block_header,
            &namespace_row_roots,
            namespace,
            last_proven_share_idx,
            namespace_boundary_proof,
        )
    }
}

#[derive(Debug)]
enum PreValidationOutput {
    /// Pre-validation concluded that verification can be completed early.
    EarlyReturn,
    /// Regular validation should be continued.
    ContinueVerification,
}

impl PreValidationOutput {
    fn is_early_return(&self) -> bool {
        matches!(self, Self::EarlyReturn)
    }
}

/// 1. Checks that blobs quantity does not exceed inclusion proofs quantity.
/// 2. Checks that row roots are non-empty for non-empty blobs slice.
/// 3. Handles the case of an empty blobs slice:
///    - if inclusion proofs are present, verification continues (unsupported blobs may be proven as skipped);
///    - if there is exactly one candidate row root, verifies absence proof and returns early;
///    - if there are multiple candidate row roots and no inclusion proof, fails closed as ambiguous absence.
fn prevalidate_blobs(
    namespace_row_roots: &[&NamespacedHash],
    blobs: &[BlobWithSender],
    namespace: Namespace,
    inclusion_proof: &[BlobProof],
    namespace_boundary_proof: &Option<NamespaceBoundaryProof>,
) -> Result<PreValidationOutput, NamespaceValidationError> {
    // This block does not contain any shares for a given namespace.
    // If no blobs have been passed, it is safe to return early.
    if namespace_row_roots.is_empty() && blobs.is_empty() {
        return Ok(PreValidationOutput::EarlyReturn);
    } else if namespace_row_roots.is_empty() && !blobs.is_empty() {
        return Err(InvalidBlobData(BlobDataError::UnexpectedBlobs));
    } else if blobs.is_empty() && !namespace_row_roots.is_empty() {
        tracing::debug!(
            row_roots = namespace_row_roots.len(),
            inclusion_proof = inclusion_proof.len(),
            has_boundary = namespace_boundary_proof.is_some(),
            "Prevalidate: empty blob list for non-empty namespace row roots"
        );
        // If inclusion proofs are present, continue with regular verification:
        // they may correspond to unsupported blobs that are intentionally skipped by extraction.
        if !inclusion_proof.is_empty() {
            return Ok(PreValidationOutput::ContinueVerification);
        }

        // Security hardening: if multiple row roots "contain" the namespace (MIN <= NAMESPACE <= MAX),
        // we cannot safely conclude full namespace absence from a single-row absence proof.
        // This can happen because parity shares use MAX namespace, broadening row namespace ranges.
        // Until completeness proof format can provide per-row absence evidence, fail closed.
        if namespace_row_roots.len() > 1 {
            tracing::warn!(
                row_roots = namespace_row_roots.len(),
                "Prevalidate: ambiguous empty namespace without inclusion proof; rejecting"
            );
            return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
        }

        let Some(NamespaceBoundaryProof {
            last_share_proof, ..
        }) = namespace_boundary_proof
        else {
            return Err(IncompleteNamespace(IncompleteNamespaceError::ProofError(
                ProofError::Missing,
            )));
        };
        // Safe due to guard above (`row_roots.len() > 1` returns MissingBlobs).
        let row_root = namespace_row_roots[0];
        return last_share_proof
            .verify_complete_namespace(row_root, &Vec::<Vec<u8>>::new(), *namespace)
            .map(|_| PreValidationOutput::EarlyReturn)
            .map_err(|e| {
                IncompleteNamespace(IncompleteNamespaceError::ProofError(ProofError::Invalid(e)))
            });
    }

    if blobs.len() > inclusion_proof.len() {
        return Err(InvalidRowProof(RowProofError::missing_proof()));
    }
    Ok(PreValidationOutput::ContinueVerification)
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
    let blob_data_read = blob.blob.accumulator();
    let first_share = blob_row_proof.first_share().map_err(InvalidRowProof)?;
    let has_signer = first_share.signer().is_some();
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
    let shares_occupied_total =
        shares_needed_for_bytes_with_signer(sequence_length as usize, has_signer);
    Ok(shares_occupied_total)
}

// Checks
// Returns number of shares covered
fn verify_skipped_blob(
    block_header: &CelestiaHeader,
    namespace_row_roots: &[&NamespacedHash],
    namespace: Namespace,
    blob_row_proof: BlobProof,
) -> Result<usize, NamespaceValidationError> {
    let first_share = blob_row_proof.first_share().map_err(InvalidRowProof)?;
    // Safety check.
    let Some(info_byte) = first_share.info_byte() else {
        return Err(InvalidBlobData(BlobDataError::NonMatchingShare));
    };
    let Some(sequence_length) = first_share.sequence_length() else {
        return Err(InvalidBlobData(BlobDataError::NonMatchingShare));
    };

    if sequence_length == 0 {
        let payload_all_zeros = first_share.payload().map(|p| p.iter().all(|b| *b == 0));
        assert!(
            payload_all_zeros.is_some(),
            "Padding share has payload with non-zero bytes",
        );
    } else {
        assert_ne!(
            info_byte.version(),
            SUPPORTED_SHARE_VERSION,
            "Bug. `verify_skipped_blob` should only be called for blobs with non supported version."
        );
    }

    // If there is more than one blob row proof, we ignore them, as we need single share.
    // We don't force prover efficiency, only correctness.
    let Some(sub_proof) = blob_row_proof.range_proofs.first() else {
        return Err(InvalidRowProof(RowProofError::missing_proof()));
    };

    let row_number = block_header.calculate_row_number_for_share(sub_proof.start_share_idx);
    let row_root = namespace_row_roots[row_number];
    let RangeProof { shares, proof, .. } = sub_proof;
    let raw_leaves = shares.iter().map(|s| s.data().as_ref()).collect::<Vec<_>>();
    proof
        .verify_range(row_root, &raw_leaves, namespace.into())
        .map_err(|e| InvalidRowProof(RowProofError::ProofError(ProofError::Invalid(e))))?;

    let shares_occupied_total = shares_needed_for_bytes_with_signer(
        sequence_length as usize,
        first_share.signer().is_some(),
    );

    Ok(shares_occupied_total)
}

// After all blobs have been verified, check namespace right boundary for completeness/censorship resistance.
// When needed, this is done with a proof for the last share of the namespace. For a valid boundary,
// the sibling on the right must belong to a strictly greater namespace.
//
// Security-critical behavior:
// * Derive boundary row as:
//   `delta = last_proven_share_idx - proof_start_in_row`, `row_idx = delta / row_len`.
// * Require row alignment: `delta % row_len == 0`.
// * Treat right sibling checks as row-local only:
//   if the boundary proof's right sibling is `PARITY_SHARE`, it only proves there are no more
//   namespace shares to the right in that row fragment. It does NOT prove global namespace end.
//   Example:
//   row r   : ... [N][N][LAST_N] | [PARITY...]
//   row r+1 : [N][N]...
//   Therefore completeness still depends on checking candidate-row position.
// * Require `row_idx` to point to the last candidate row in `namespace_row_roots`.
//   If it points earlier, later candidate rows could still contain this namespace, so return `MissingBlobs`.
//
// Parameters:
// * `block_header` is trusted.
// * `namespace_row_roots` is trusted (derived from verified DAH).
// * `namespace` is trusted rollup parameter.
// * `last_proven_share_idx` is derived by the verifier from validated shares.
// * `namespace_boundary_proof` may be `None` only when last proven share is the last share of the last candidate row.
fn check_namespace_end_boundary(
    block_header: &CelestiaHeader,
    namespace_row_roots: &[&NamespacedHash],
    namespace: Namespace,
    last_proven_share_idx: usize,
    namespace_boundary_proof: Option<NamespaceBoundaryProof>,
) -> Result<(), NamespaceValidationError> {
    let last_share_last_row_idx = namespace_row_roots.len() * block_header.row_length() - 1;
    // if the last proven share ended exactly on the last index of the last row,
    // meaning there are no shares from this namespace anymore,
    // because the next row does not contain given namespace at all.
    assert!(last_proven_share_idx <= last_share_last_row_idx, "bad math");
    if last_proven_share_idx < last_share_last_row_idx {
        // Verifying the completeness of the namespace.
        let Some(NamespaceBoundaryProof {
            last_share_proof,
            last_share,
        }) = namespace_boundary_proof
        else {
            return Err(IncompleteNamespace(IncompleteNamespaceError::ProofError(
                ProofError::Missing,
            )));
        };
        // Determine the row of the boundary proof from the trusted `last_proven_share_idx`.
        // Security invariant: the boundary proof must terminate the namespace in the last
        // candidate row root. Otherwise, rows after `row_idx` could still contain this namespace.
        // Upsize everything to u64, even though zkVM is 32 bit, and such big namespace rows are highly unlikely,
        // better to be on the safe side.
        let row_len = block_header.row_length() as u64;
        let proof_start_in_row = last_share_proof.start_idx() as u64;
        let last_proven_share_idx_u64 = last_proven_share_idx as u64;
        if last_proven_share_idx_u64 < proof_start_in_row {
            tracing::error!(
                last_proven_share_idx = last_proven_share_idx_u64,
                proof_start_in_row,
                "MissingBlobs in check_namespace_end_boundary: proven index is before proof start in row"
            );
            return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
        }

        let delta = last_proven_share_idx_u64
            .checked_sub(proof_start_in_row)
            .expect("checked above");
        let row_idx_u64 = delta.checked_div(row_len).expect("row_len cannot be 0");
        let rem = delta % row_len;
        if rem != 0 {
            return Err(IncompleteNamespace(
                IncompleteNamespaceError::corrupted_proof(),
            ));
        }
        let row_idx = usize::try_from(row_idx_u64)
            .map_err(|_| IncompleteNamespace(IncompleteNamespaceError::corrupted_proof()))?;
        if row_idx >= namespace_row_roots.len() {
            return Err(IncompleteNamespace(
                IncompleteNamespaceError::corrupted_proof(),
            ));
        }
        let last_row_idx = namespace_row_roots
            .len()
            .checked_sub(1)
            .expect("row roots cannot be empty in this branch");
        if row_idx != last_row_idx {
            tracing::error!(
                row_idx,
                last_row_idx,
                "MissingBlobs in check_namespace_end_boundary: boundary proof does not target last namespace row"
            );
            return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
        }
        let last_row_root = namespace_row_roots[last_row_idx];

        let Some(raw_leaves) = last_share.as_ref().map(|s| vec![s.data()]) else {
            return Err(IncompleteNamespace(
                IncompleteNamespaceError::corrupted_proof(),
            ));
        };
        if let Err(e) = last_share_proof.verify_range(last_row_root, &raw_leaves, *namespace) {
            return Err(IncompleteNamespace(IncompleteNamespaceError::ProofError(
                ProofError::Invalid(e),
            )));
        };
        let Some(lrs) = last_share_proof.leftmost_right_sibling() else {
            return Err(IncompleteNamespace(IncompleteNamespaceError::ProofError(
                ProofError::Corrupted,
            )));
        };
        if *namespace >= lrs.min_namespace() {
            tracing::error!(
                namespace = ?namespace,
                right_sibling_min = ?lrs.min_namespace(),
                "MissingBlobs in check_namespace_end_boundary: right sibling is not strictly greater"
            );
            return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
