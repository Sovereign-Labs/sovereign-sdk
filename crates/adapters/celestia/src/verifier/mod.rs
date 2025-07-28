pub mod address;
pub mod proofs;

use std::cmp::Ordering;

use borsh::{BorshDeserialize, BorshSerialize};
use celestia_types::nmt::{Namespace, NamespacedHash, NS_SIZE};
use nmt_rs::NamespacedSha2Hasher;
use sov_rollup_interface::da::{self, DaSpec, RelevantBlobs, RelevantProofs};

use self::address::CelestiaAddress;
use self::proofs::*;
use crate::shares::shares_needed_for_bytes;
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
            relevant_proofs.batch.inclusion_proof,
            relevant_proofs.batch.completeness_proof,
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
            relevant_proofs.proof.inclusion_proof,
            relevant_proofs.proof.completeness_proof,
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
    /// * the slice of [`BlobsWithSender`] that has been extracted. Not trusted.
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

/// 1. Checks that blobs quantity matches inclusion proofs quantity.
/// 2. Checks that row roots are non-empty for non-empty blobs slice.
/// 3. Handles the case of an empty blobs slice and verifies absence proof.
///    In this case returns `Ok(None)` and the caller can exit early.
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
        // We get a list of all row roots that "contain" our namespace (i.e. MIN <= NAMESPACE <= MAX)
        // it's possible that there's a row whose root "contains" our namespace even though no shares from our namespace are actually present in that row.
        // For that to be the case, the row needs to contain shares from at least two different namespaces,
        // one, which is less than our namespace, and one which is greater.
        // Because shares are ordered by namespace, there can only be at most one such row.
        // The row before that one will only have shares that are strictly less than our namespace, and the row after will only have shares that are strictly greater.

        if namespace_row_roots.len() > 1 {
            return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
        }
        let row_root = namespace_row_roots[0];
        // Verifying that there are no shares in this single row.
        let Some(NamespaceBoundaryProof {
            last_share_proof, ..
        }) = namespace_boundary_proof
        else {
            return Err(IncompleteNamespace(IncompleteNamespaceError::ProofError(
                ProofError::Missing,
            )));
        };
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

/// 1. Checks that given blob's first share starts immediately after the previous blob.
///    (i.e. that there are no gaps between blobs)
/// 2. That the passed `BlobProof` is valid
/// 3. That the passed `BlobProof` only covers shares touched by the rollup. At least 1 share is always verified.
/// 4. That the shares in `BlobProof` match the data in `Blob`. This includes the signer and each byte of the payload that was actually read by the rollup.
///
/// For the very first blob (`blob_idx == 0`) it also checks the left boundary of the whole completeness check.
/// In other words, it checks that the share preceding the first blob is from another namespace.
///
/// Returns a number of shares checked (proven and skipped)
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
    let num_shares_to_prove = shares_needed_for_bytes(blob_data_read.len()).max(1);
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
    let shares_occupied_total = shares_needed_for_bytes(sequence_length as usize);
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

    let shares_occupied_total = shares_needed_for_bytes(sequence_length as usize);

    Ok(shares_occupied_total)
}

// After all blobs have been verified, we need to check that there are no more blobs in the namespace.
// It does it by explicitly checking proof of the last share. For this proof, the leaf on the right must be from another namespace.
// Parameters:
// * `block_header` is a trusted input parameter.
// * `namespace_row_roots` is trusted, as it should've been trustlessly derived from the block header.
// * `namespace` is a trusted rollup parameter.
// * `last_proven_share_idx` is trusted and should be properly derived by the caller.\
// * `namespace_boundary_proof` is allowed to be None if the last proven share is the last share in the row.
//    This is checked
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
        // Upsize everything to u64, even though zkVM is 32 bit, and such big namespace rows are highly unlikely,
        // better to be on the safe side.
        let last_share_proof_start_idx = (namespace_row_roots.len() as u64 - 1)
            .checked_mul(block_header.row_length() as u64)
            .expect("Square overflow")
            .checked_add(last_share_proof.start_idx() as u64)
            .expect("Square overflow");

        // Last proven share should match index of the proof
        // This index is trusted, because it is derived from sequence length,
        // which is tied to the row root.
        match (last_proven_share_idx as u64).cmp(&last_share_proof_start_idx) {
            Ordering::Less => {
                return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
            }
            Ordering::Equal => {}
            Ordering::Greater => {
                return Err(IncompleteNamespace(
                    IncompleteNamespaceError::corrupted_proof(),
                ));
            }
        }
        let last_row_root = namespace_row_roots
            .last()
            .expect("Empty namespace row roots have been checked before");

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
            return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
        }
    }
    Ok(())
}
