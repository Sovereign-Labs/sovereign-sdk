use serde::{Deserialize, Serialize};

use crate::shares::is_tail_padding;
use crate::types::NamespaceValidationError::{
    IncompleteNamespace, InvalidBlobData, InvalidRowProof,
};
use crate::types::{
    BlobDataError, IncompleteNamespaceError, NamespaceValidationError, RowProofError,
    SUPPORTED_SHARE_VERSION,
};

/// BlobProof contains proof of each range.
/// Ranges are different and not as a single, because a blob can span across rows,
/// so it will have different proofs, as each row has a separate proof.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BlobProof {
    pub(crate) range_proofs: Vec<RangeProof>,
}

impl BlobProof {
    // Ensures that range proof covers continuous range.
    // Different blobs will never overlap each other in the same share, because:
    // Each blob starts from a new share, called a first share in the sequence,
    // And it can span to several shares, where the following share is called "Continuation Share".
    // Remaining bytes are padded with zeroes:
    // > The remaining SHARE_SIZE-NAMESPACE_SIZE-SHARE_INFO_BYTES-SEQUENCE_BYTES bytes are filled with 0
    // From
    // https://celestiaorg.github.io/celestia-app/specs/shares.html#padding
    pub(crate) fn enforce_continuity(&self) -> Result<(), RowProofError> {
        for i in 1..self.range_proofs.len() {
            let left_idx = i
                .checked_sub(1)
                .expect("bug: loop should be started from 1");
            let l = &self.range_proofs[left_idx];
            let r = &self.range_proofs[i];
            let left_end = l.start_share_idx.saturating_add(l.shares.len());
            if r.start_share_idx != left_end {
                tracing::warn!(
                    "sub-proofs are not contiguous between proofs {} and {}",
                    left_idx,
                    i
                );
                return Err(RowProofError::missing_proof());
            }
        }
        Ok(())
    }

    pub(crate) fn first_share(&self) -> Result<&celestia_types::Share, RowProofError> {
        self.range_proofs
            .first()
            .and_then(|range_proof| range_proof.shares.first())
            .ok_or(RowProofError::WrongNumberOfShares {
                expected: 1,
                actual: 0,
            })
    }

    /// Returns true if this proof is for a supported blob.
    pub(crate) fn is_supported_blob(&self) -> Result<bool, NamespaceValidationError> {
        let first_share = self.first_share().map_err(InvalidRowProof)?;
        let Some(info_byte) = first_share.info_byte() else {
            return Err(InvalidBlobData(BlobDataError::NonMatchingShare));
        };
        Ok(info_byte.version() == SUPPORTED_SHARE_VERSION && !is_tail_padding(first_share))
    }

    fn get_first_sub_proof(&self) -> Result<&RangeProof, RowProofError> {
        self.range_proofs
            .first()
            .ok_or_else(RowProofError::missing_proof)
    }

    /// Check that this blob proof is for the first blob in the namespace.
    /// Returns row-adjusted index of the first blob.
    pub(crate) fn verify_left_boundary(
        &self,
        block_header: &crate::CelestiaHeader,
        namespace: celestia_types::nmt::Namespace,
    ) -> Result<usize, NamespaceValidationError> {
        let first_sub_proof = self.get_first_sub_proof().map_err(InvalidRowProof)?;
        let blob_proof_range_start = first_sub_proof.start_share_idx;

        // 1. It should point to the first row.
        // The start range is row adjusted, but namespace relative. So it should be always row zero,
        // Even if first row that contains this namespace is not the first row in the block.
        let row_number = block_header.calculate_row_number_for_share(blob_proof_range_start);
        if row_number != 0 {
            return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
        }

        // 2. Check consistency between `start_share_idx` and `proof.start_idx()`
        // Because it is a first row, total start_share_idx, which is index in the namespace.
        // Should match proof start_idx, which is the index inside the row.
        // Note: converting both to u64 for correct validation and making this variable trusted.
        if first_sub_proof.start_share_idx as u64 != first_sub_proof.proof.start_idx() as u64 {
            return Err(InvalidRowProof(RowProofError::WrongStartShareIndex {
                expected: first_sub_proof.start_share_idx,
                actual: first_sub_proof.proof.start_idx() as usize,
            }));
        }

        // 3. If the first blob starts not from the beginning of the row (start_idx() == 0),
        // we need to prove that there are no skipped shares from the namespace.
        // Otherwise, we safely know that the first column of the related row contains this proof,
        // so there are blobs that have been skipped.
        if first_sub_proof.proof.start_idx() > 0 {
            let Some(rls) = first_sub_proof.proof.rightmost_left_sibling() else {
                return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
            };

            // rightmost left sibling should have namespace that strictly smaller than ours.
            if rls.max_namespace() >= *namespace {
                return Err(IncompleteNamespace(IncompleteNamespaceError::MissingBlobs));
            }
        }

        Ok(blob_proof_range_start)
    }

    /// Verifies that this blob proof is adjacent to the previous without any gaps or overlaps.
    /// Returns row-adjusted index of the first blob.
    pub(crate) fn verify_continuity(
        &self,
        last_validated_share_idx: usize,
    ) -> Result<usize, NamespaceValidationError> {
        let expected_range_start = last_validated_share_idx
            .checked_add(1)
            .expect("Share index overflow");

        let first_sub_proof = self.get_first_sub_proof().map_err(InvalidRowProof)?;
        let blob_proof_range_start = first_sub_proof.start_share_idx;

        if blob_proof_range_start != expected_range_start {
            return Err(InvalidRowProof(RowProofError::WrongStartShareIndex {
                expected: expected_range_start,
                actual: blob_proof_range_start,
            }));
        }

        Ok(blob_proof_range_start)
    }
}

/// Proof of range inside a single row
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct RangeProof {
    pub(crate) shares: Vec<celestia_types::Share>,
    pub(crate) proof: celestia_types::nmt::NamespaceProof,
    // Index in the namespace, aligned for data row, not for namespace
    // Index is relative to the first row which contains the given namespace.
    pub(crate) start_share_idx: usize,
}

#[cfg(feature = "native")]
pub(crate) fn new_inclusion_proof(
    header: &crate::CelestiaHeader,
    rollup_data: &crate::types::NamespaceRelevantData,
    blobs: &[crate::types::BlobWithSender],
) -> Vec<BlobProof> {
    let mut needed_share_ranges = Vec::new();

    let mut prev_range_end: Option<usize> = None;
    let flat_shares = rollup_data
        .data
        .rows
        .iter()
        .flat_map(|r| r.shares.iter())
        .collect::<Vec<_>>();

    // Extract the positions of each blob up to the number of shares actually used.
    for blob in blobs.iter() {
        let range = blob.range_in_namespace.clone();
        let relevant_len = blob.blob.accumulator().len();
        let relevant_end =
            range.start + crate::shares::shares_needed_for_bytes(relevant_len).max(1);
        // Guard in depth - should never be false unless either shares_needed_for_bytes() is bugged
        // or we read more bytes from the blob than the range has shares (e.g. `BlobWithIter`'s range
        // initialisation is bugged)
        assert!(
            relevant_end <= range.end,
            "relevant_end > range.end: {} > {}",
            relevant_end,
            range.end
        );

        if range.start != 0 {
            let prev_range_end = prev_range_end.unwrap_or(0);
            if range.start != prev_range_end {
                let skipped_blob_ranges = build_ranges_to_prove_for_skipped_blobs(
                    prev_range_end..range.start,
                    &flat_shares,
                );
                needed_share_ranges.extend(skipped_blob_ranges);
            }
        }

        needed_share_ranges.push(range.start..relevant_end);
        prev_range_end = Some(range.end);
    }

    let end_of_ns = flat_shares.len();
    if prev_range_end.is_some() && prev_range_end != Some(end_of_ns) {
        let skipped_blob_ranges = build_ranges_to_prove_for_skipped_blobs(
            prev_range_end.unwrap()..end_of_ns,
            &flat_shares,
        );
        needed_share_ranges.extend(skipped_blob_ranges);
    }

    let row_roots = header
        .get_row_roots_for_namespace(rollup_data.namespace)
        .cloned()
        .collect::<Vec<_>>();

    sub_namespace_inclusion_proofs(
        header.row_length(),
        &rollup_data.data,
        rollup_data.namespace,
        &needed_share_ranges,
        &row_roots,
    )
}

#[cfg(feature = "native")]
fn build_ranges_to_prove_for_skipped_blobs(
    inner_range: std::ops::Range<usize>,
    flat_namespace_shares: &[&celestia_types::Share],
) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = inner_range.start;
    while start < inner_range.end {
        let start_share = flat_namespace_shares[start];
        // Only a single share is enough
        ranges.push(start..(start + 1));
        let info_byte = start_share.info_byte().expect("Bug. Missing info byte");
        let sequence_length = start_share
            .sequence_length()
            .expect("Bug. Missing sequence length");
        if sequence_length == 0 {
            let payload_all_zeros = start_share.payload().map(|p| p.iter().all(|b| *b == 0));
            assert!(
                payload_all_zeros.is_some(),
                "Padding share {} has payload with non-zero bytes: {:?}",
                start,
                start_share.payload().map(hex::encode),
            );
        } else {
            assert_ne!(
                info_byte.version(),
                SUPPORTED_SHARE_VERSION,
                "Skipping version 1, bug!"
            );
        }
        let shares_in_blob =
            crate::shares::shares_needed_for_bytes(sequence_length as usize).max(1);
        start += shares_in_blob;
    }

    ranges
}

/// The namespace is a contiguous set of shares from the EDS (Extended Data Square).
/// It's arranged in the rows corresponding to the EDS rows, with the first and last row included
/// being not necessarily the full width. E.g.:
///```ascii
/// [ A1, A2, B1, B2]
/// [ B3, B4, B5, B6]
/// [ B7, C1, C2, C3]
/// ```
///
/// Here, the NamespaceData for namespace B will be shaped as [[B1, B2], [B3, B4, B5, B6], [B7]].
/// [`NamespaceData`] also includes row proofs for every included (sub)row. These are merkle
/// proofs whose root is the EDS row root - for example, in the case above, B3-B6 make a complete row
/// so the merkle proof is trivial, while the two partial rows B1-B2 and B7 respectively each get a
/// merkle proof that can be used to verify inclusion in the EDS row. (In turn, the extended data
/// header contains the proofs necessary to verify the inclusion of each row root.)
///
/// This function proves the inclusion of specific shares within that namespace, as determined by
/// `ranges_to_prove`. It uses the known namespace shares outside of the desired range, alongside
/// the existing proof for the shares outside the namespace to generate a narrower merkle range
/// proof.
///
/// Because the proofs are based on row proofs and are rooted in the row roots, ranges that span
/// multiple rows get separate proofs for each fragment on a different row.
/// E.g. passing a B2-B3 range using the same example above would generate two proofs, one for
/// share B2 and one for share B3; or, hypothetically, passing a B2-B7 range would generate three
/// separate proofs: one for B2, one for the B3-B6 row, and one for B7. This is why inclusion proof is
/// itself a vec of blob proofs.
#[cfg(feature = "native")]
fn sub_namespace_inclusion_proofs(
    row_length: usize,
    namespace_data: &celestia_types::row_namespace_data::NamespaceData,
    namespace: celestia_types::nmt::Namespace,
    blob_ranges_to_prove: &[std::ops::Range<usize>],
    row_roots: &[celestia_types::nmt::NamespacedHash],
) -> Vec<BlobProof> {
    #[cfg(debug_assertions)]
    {
        // using regular asser, as whole block is wrapped in debug_assertions
        assert!(
            check_ranges_sorted(blob_ranges_to_prove),
            "Ranges non-sorted or overlapping"
        );
    }
    // Should be sorted and non-overlapping
    let mut output = Vec::with_capacity(blob_ranges_to_prove.len());

    // Shares in the first row are aligned right
    let first_row_offset = if !namespace_data.rows.is_empty() {
        let first_row = &namespace_data.rows[0];
        row_length
            .checked_sub(first_row.shares.len())
            .expect("Row cannot be larger that square size")
    } else {
        0
    };

    for blob_range in blob_ranges_to_prove {
        let per_row_sub_ranges = split_blob_range_by_rows(blob_range, first_row_offset, row_length);
        let mut current_blob_proof: BlobProof = BlobProof {
            range_proofs: Vec::new(),
        };

        for blob_sub_range in per_row_sub_ranges {
            let row_num = blob_sub_range
                .start
                .checked_div(row_length)
                .expect("row_length cannot be 0");

            let namespace_row = &namespace_data.rows[row_num];

            let mut row_relative_start = blob_sub_range.start % row_length;
            let mut row_relative_end = row_relative_start
                .checked_add(blob_sub_range.len())
                .unwrap();
            if row_num == 0 {
                row_relative_start = row_relative_start.checked_sub(first_row_offset).unwrap();
                row_relative_end = row_relative_end.checked_sub(first_row_offset).unwrap();
            }

            let shares = &namespace_row.shares[row_relative_start..row_relative_end];

            let row_proof = namespace_row
                .proof
                .narrow_range(
                    &namespace_row.shares[0..row_relative_start],
                    &namespace_row.shares[row_relative_end..namespace_row.shares.len()],
                    namespace.into(),
                )
                .unwrap();

            let raw_leaves = shares
                .iter()
                .map(|s| {
                    debug_assert_eq!(s.namespace(), namespace, "share namespace mismatch");
                    s.to_vec()
                })
                .collect::<Vec<_>>();

            let this_row_root = &row_roots[row_num];

            tracing::trace!("verify while building: row_roots[{}]", row_num,);
            debug_assert!(
                this_row_root.contains::<celestia_types::nmt::NamespacedSha2Hasher>(*namespace),
                "wrong row root for the namespace"
            );
            row_proof
                .verify_range(this_row_root, &raw_leaves, namespace.into())
                .expect("invalid proof self-check");

            current_blob_proof.range_proofs.push(RangeProof {
                shares: shares.to_vec(),
                proof: row_proof.into(),
                start_share_idx: blob_sub_range.start,
            });
        }
        if !current_blob_proof.range_proofs.is_empty() {
            output.push(current_blob_proof);
        } else {
            panic!("Empty proof for blob range: {blob_range:?}");
        }
    }
    output
}

#[cfg(all(feature = "native", any(debug_assertions, test, bench)))]
fn check_ranges_sorted(ranges: &[std::ops::Range<usize>]) -> bool {
    if ranges.is_empty() {
        return true;
    }

    for i in 0..ranges.len().saturating_sub(1) {
        if ranges[i].start > ranges[i + 1].start {
            return false;
        }

        if ranges[i].end > ranges[i + 1].start {
            return false;
        }
    }

    true
}

/// Converts namespace relative range into the set of per-row ranges with absolute coordinates inside data square
/// Blob range is a "flat" range over the whole namespace.
/// It can span across several rows.
/// Returns sub-ranges adjusted to the offset.
#[cfg(feature = "native")]
#[allow(clippy::single_range_in_vec_init)]
fn split_blob_range_by_rows(
    ns_range: &std::ops::Range<usize>,
    first_row_offset: usize,
    square_size: usize,
) -> Vec<std::ops::Range<usize>> {
    if ns_range.is_empty() {
        return Vec::new();
    }
    let std::ops::Range {
        start: start_relative,
        end: end_relative,
    } = *ns_range;

    let start_absolute = start_relative
        .checked_add(first_row_offset)
        .expect("Index overflow");
    let end_absolute = end_relative
        .checked_add(first_row_offset)
        .expect("Index overflow");

    // Function to calculate which row a flat index belongs to
    let calculate_row = |absolute_idx: usize| -> usize {
        if absolute_idx < square_size {
            return 0;
        }
        absolute_idx
            .checked_div(square_size)
            .expect("square_size cannot be 0")
    };

    let start_row = calculate_row(start_absolute);
    // end is exclusive, and range is non-empty as checked above.
    let end_row = calculate_row(end_absolute - 1);

    // No row crossing boundary.
    if start_row == end_row {
        return vec![start_absolute..end_absolute];
    }

    let mut output = Vec::new();
    let mut start = start_absolute;
    let mut current_row = start_row;
    while current_row <= end_row {
        // The last index of row x is for square n: `(x * n) + (n - 1)`.
        // We need an exclusive range, so it is simply `x * n + n`
        let row_end = current_row
            .checked_mul(square_size)
            .expect("Row index overflow")
            .checked_add(square_size)
            .expect("Row index overflow");

        let end = if current_row < end_row {
            row_end
        } else {
            // Trimming in the case of the last row.
            end_absolute
        };
        output.push(start..end);
        start = end;
        current_row += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::*;

    const SQUARE_SIZE: usize = 4;

    #[test]
    fn single_share_range_always_single_sub_range() {
        for end in 1usize..100 {
            let start = end.checked_sub(1).expect("end cannot be 0");
            let range = Range { start, end };
            for first_row_offset in 0..SQUARE_SIZE {
                let sub_ranges = split_blob_range_by_rows(&range, first_row_offset, SQUARE_SIZE);
                assert_eq!(sub_ranges.len(), 1);
                let expected_range = Range {
                    start: range.start + first_row_offset,
                    end: range.end + first_row_offset,
                };
                assert_eq!(sub_ranges[0], expected_range);
            }
        }
    }

    #[test]
    fn range_smaller_than_square_size_non_crossing_row_boundary() {
        let cases = vec![
            (Range { start: 0, end: 4 }, 0),
            (Range { start: 1, end: 2 }, 1),
            (Range { start: 9, end: 11 }, 0),
            // Crossing mod square size boundary, but offset adjusts it.
            (Range { start: 6, end: 8 }, 3),
        ];

        for (range, first_row_offset) in cases {
            let sub_ranges = split_blob_range_by_rows(&range, first_row_offset, SQUARE_SIZE);
            assert_eq!(sub_ranges.len(), 1);
            verify_split(&range, &sub_ranges, first_row_offset);
        }
    }

    #[test]
    fn range_crossing_row_boundary() {
        let cases = vec![
            // Small range
            (Range { start: 2, end: 5 }, 0, 2),
            // Small range with offset
            (Range { start: 0, end: 3 }, 2, 2),
            (Range { start: 4, end: 7 }, 2, 2),
            // Big range
            (Range { start: 0, end: 6 }, 0, 2),
            (Range { start: 0, end: 9 }, 0, 3),
            // Big range with offset
            (Range { start: 0, end: 7 }, 2, 3),
        ];

        for (range, first_row_offset, expected_sub_ranges_len) in cases {
            let sub_ranges = split_blob_range_by_rows(&range, first_row_offset, SQUARE_SIZE);
            assert_eq!(sub_ranges.len(), expected_sub_ranges_len);
            verify_split(&range, &sub_ranges, first_row_offset);
        }
    }

    fn verify_split(range: &Range<usize>, sub_ranges: &[Range<usize>], offset: usize) {
        assert!(
            !sub_ranges.is_empty(),
            "There should be at least one sub range for given range"
        );
        // sub ranges are consecutive and sorted
        assert!(check_ranges_sorted(sub_ranges));

        for sub_range in sub_ranges {
            // each sub-range is inside the source range after excluding offset.
            let start = sub_range.start.checked_sub(offset).expect("Index overflow");
            range.contains(&start);
            let end = sub_range.end.checked_sub(offset).expect("Index overflow");
            assert!(end <= range.end);
        }
        let first_start = sub_ranges
            .first()
            .unwrap()
            .start
            .checked_sub(offset)
            .expect("Index overflow");
        let last_end = sub_ranges
            .last()
            .unwrap()
            .end
            .checked_sub(offset)
            .expect("Index overflow");
        assert_eq!(first_start, range.start);
        assert_eq!(last_end, range.end);
    }
}
