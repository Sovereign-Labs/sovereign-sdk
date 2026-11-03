use celestia_types::nmt::{Namespace, NamespacedHash, NS_SIZE};
use nmt_rs::NamespacedSha2Hasher;

use crate::header::CelestiaHeader;
use crate::proofs::{BlobProof, RangeProof};
use crate::shares::shares_needed_for_bytes_with_signer;
use crate::types::NamespaceValidationError::{
    IncompleteNamespace, InvalidBlobData, InvalidRowProof,
};
/// Namespaces a rollup reads on Celestia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RollupParams {
    pub rollup_batch_namespace: Namespace,
    pub rollup_proof_namespace: Namespace,
}

use crate::types::{
    BlobDataError, IncompleteNamespaceError, NamespaceBoundaryProof, NamespaceValidationError,
    ProofError, RowProofError,
};

/// Walk one namespace: continuity, skipped-blob proofs, and the right-hand boundary.
///
/// `is_supported` selects the shares this adapter reads. Every other share is proven as
/// skipped. `authenticate` checks one supported blob and returns how many shares it occupies.
pub fn verify_namespace<B>(
    block_header: &CelestiaHeader,
    blobs: &[B],
    namespace: Namespace,
    inclusion_proof: Vec<BlobProof>,
    namespace_boundary_proof: Option<NamespaceBoundaryProof>,
    is_supported: impl Fn(&celestia_types::Share) -> bool,
    mut authenticate: impl FnMut(&B, BlobProof) -> Result<usize, NamespaceValidationError>,
) -> Result<(), NamespaceValidationError> {
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

    let mut last_validated_share_idx: Option<usize> = None;
    let mut blob_proofs_iter = inclusion_proof.into_iter().peekable();
    let mut blobs_iter = blobs.iter().peekable();
    let mut blob_idx = 0;

    while blobs_iter.peek().is_some() || blob_proofs_iter.peek().is_some() {
        let Some(blob_row_proof) = blob_proofs_iter.next() else {
            return Err(InvalidRowProof(RowProofError::missing_proof()));
        };

        let blob_proof_range_start = {
            let blob_proof_range_start = if blob_idx == 0 {
                blob_row_proof.verify_left_boundary(block_header, namespace)?
            } else {
                let last_from_previous_blob = last_validated_share_idx
                    .expect("Bug: caller didn't set last_validated_share_idx");
                blob_row_proof.verify_continuity(last_from_previous_blob)?
            };
            blob_row_proof
                .enforce_continuity()
                .map_err(InvalidRowProof)?;
            blob_proof_range_start
        };

        for sub_proof in &blob_row_proof.range_proofs {
            let expected = sub_proof.start_share_idx % block_header.row_length();
            let actual = sub_proof.proof.start_idx() as usize;
            if actual != expected {
                return Err(InvalidRowProof(RowProofError::WrongStartShareIndex {
                    expected,
                    actual,
                }));
            }
        }

        let shares_checked = if blob_row_proof.is_supported_blob(&is_supported)? {
            let Some(blob) = blobs_iter.next() else {
                return Err(InvalidBlobData(BlobDataError::MoreProofsThanBlobs));
            };
            authenticate(blob, blob_row_proof)?
        } else {
            verify_skipped_blob(
                block_header,
                &namespace_row_roots,
                namespace,
                blob_row_proof,
                &is_supported,
            )?
        };
        last_validated_share_idx = Some(match last_validated_share_idx {
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

#[derive(Debug)]
pub enum PreValidationOutput {
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
pub fn prevalidate_blobs<B>(
    namespace_row_roots: &[&NamespacedHash],
    blobs: &[B],
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

// Checks
// Returns number of shares covered
fn verify_skipped_blob(
    block_header: &CelestiaHeader,
    namespace_row_roots: &[&NamespacedHash],
    namespace: Namespace,
    blob_row_proof: BlobProof,
    is_supported: &impl Fn(&celestia_types::Share) -> bool,
) -> Result<usize, NamespaceValidationError> {
    let first_share = blob_row_proof.first_share().map_err(InvalidRowProof)?;
    // Safety check.
    if first_share.info_byte().is_none() {
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
        assert!(
            !is_supported(first_share),
            "Bug. `verify_skipped_blob` should only be called for unsupported shares."
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
pub fn check_namespace_end_boundary(
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
