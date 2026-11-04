use celestia_types::nmt::{Namespace, NamespacedHash, NS_SIZE};
use celestia_types::Share;
use nmt_rs::simple_merkle::error::RangeProofError;
use nmt_rs::simple_merkle::proof::Proof;
use nmt_rs::simple_merkle::tree::MerkleHash;
use nmt_rs::simple_merkle::utils::{compute_num_left_siblings, compute_tree_size};
use nmt_rs::{NamespacedSha2Hasher, TmSha2Hasher};
use sha2::{Digest, Sha256};
use sov_celestia_common::address::CelestiaAddress;
use sov_celestia_common::header::CelestiaHeader;
use sov_celestia_common::proofs::BlobProof;
use sov_celestia_common::shares::{is_tail_padding, shares_needed_for_bytes_with_signer};
use sov_celestia_common::types::NamespaceValidationError::{InvalidBlobData, InvalidRowProof};
use sov_celestia_common::types::{
    BlobDataError, NamespaceBoundaryProof, NamespaceType, NamespaceValidationError, ProofError,
    RowProofError, ValidationError,
};
use sov_celestia_common::verifier::{verify_namespace, RollupParams};
use sov_rollup_interface::da::{BlobReaderTrait, DaVerifier, RelevantBlobs, RelevantProofs};

use crate::blob::FibreBlob;
use crate::proof::{FibreBlobProof, FibreProofError, FibreRowsProof};
use crate::spec::CelestiaFibreSpec;

pub const FIBRE_SHARE_VERSION: u8 = 2;
pub const FIBRE_BLOB_VERSION: u32 = 0;
pub const FIBRE_K: usize = 4096;
pub const FIBRE_N: usize = 12288;
pub const FIBRE_HEADER_LEN: usize = 5;
pub const FIBRE_ROW_ALIGN: usize = 64;
pub const FIBRE_MAX_BLOB_SIZE: usize = 1 << 27;
pub const FIBRE_POINTER_LEN: u32 = 36;

const _: () = assert!(FIBRE_K.is_power_of_two() && (FIBRE_K + FIBRE_N).is_power_of_two());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FibrePointer {
    pub blob_version: u32,
    pub commitment: [u8; 32],
}

pub fn parse_v2_pointer(share: &Share) -> Result<FibrePointer, FibreProofError> {
    let info_byte = share.info_byte().ok_or(FibreProofError::InvalidPointer)?;
    if info_byte.version() != FIBRE_SHARE_VERSION
        || !info_byte.is_sequence_start()
        || share.sequence_length() != Some(FIBRE_POINTER_LEN)
    {
        return Err(FibreProofError::InvalidPointer);
    }
    let payload = share
        .payload()
        .and_then(|payload| payload.get(..FIBRE_POINTER_LEN as usize))
        .ok_or(FibreProofError::InvalidPointer)?;
    let (version, commitment) = payload.split_at(4);
    let blob_version = u32::from_be_bytes(
        version
            .try_into()
            .map_err(|_| FibreProofError::InvalidPointer)?,
    );
    if blob_version != FIBRE_BLOB_VERSION {
        return Err(FibreProofError::UnsupportedBlobVersion(blob_version));
    }
    Ok(FibrePointer {
        blob_version,
        commitment: commitment
            .try_into()
            .map_err(|_| FibreProofError::InvalidPointer)?,
    })
}

pub fn row_size_for(data_len: usize) -> Option<usize> {
    let blob_len = data_len.checked_add(FIBRE_HEADER_LEN)?;
    if blob_len > FIBRE_MAX_BLOB_SIZE {
        return None;
    }
    Some(blob_len.div_ceil(FIBRE_K).next_multiple_of(FIBRE_ROW_ALIGN))
}

pub fn rows_needed_for_prefix(prefix_len: usize, row_size: usize) -> usize {
    FIBRE_HEADER_LEN
        .saturating_add(prefix_len)
        .div_ceil(row_size)
        .max(1)
}

pub fn supported_share(share: &Share) -> bool {
    share
        .info_byte()
        .is_some_and(|info| info.version() == FIBRE_SHARE_VERSION)
        && !is_tail_padding(share)
}

pub fn verify_fibre_prefix(
    commitment: &[u8; 32],
    proof: &FibreRowsProof,
    expected_prefix: &[u8],
    expected_total_len: usize,
) -> Result<(), FibreProofError> {
    let row_size = proof.row_size as usize;
    if row_size == 0
        || !row_size.is_multiple_of(FIBRE_ROW_ALIGN)
        || !proof.rows.len().is_multiple_of(row_size)
    {
        return Err(FibreProofError::RowSizeInvalid(proof.row_size));
    }
    let actual_rows = proof.rows.len() / row_size;
    let expected_rows = rows_needed_for_prefix(expected_prefix.len(), row_size);
    if actual_rows != expected_rows || actual_rows > FIBRE_K {
        return Err(FibreProofError::RowCountMismatch {
            expected: expected_rows.min(FIBRE_K),
            actual: actual_rows,
        });
    }
    if expected_prefix.len() > expected_total_len {
        return Err(FibreProofError::PrefixMismatch);
    }

    let hasher = TmSha2Hasher::new();
    let leaves: Vec<[u8; 32]> = proof
        .rows
        .chunks_exact(row_size)
        .map(|row| hasher.hash_leaf(row))
        .collect();
    let row_root = fold_range_root(&leaves, &proof.siblings)?;
    if sha256_pair(&row_root, &proof.rlc_root) != *commitment {
        return Err(FibreProofError::CommitmentMismatch);
    }

    let (header, data) = proof.rows.split_at(FIBRE_HEADER_LEN);
    if header[0] != 0 {
        return Err(FibreProofError::HeaderInvalid);
    }
    let data_size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if data_size == 0 || row_size_for(data_size).is_none() {
        return Err(FibreProofError::HeaderInvalid);
    }
    if data_size != expected_total_len {
        return Err(FibreProofError::DataSizeMismatch {
            expected: expected_total_len as u64,
            actual: data_size as u64,
        });
    }
    if row_size_for(data_size) != Some(row_size) {
        return Err(FibreProofError::RowSizeInvalid(proof.row_size));
    }
    if data.get(..expected_prefix.len()) != Some(expected_prefix) {
        return Err(FibreProofError::PrefixMismatch);
    }
    Ok(())
}

fn sha256_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

fn fold_range_root(
    leaves: &[[u8; 32]],
    siblings: &[[u8; 32]],
) -> Result<[u8; 32], FibreProofError> {
    if leaves.is_empty() {
        return Err(FibreProofError::RangeProofInvalid(
            RangeProofError::NoLeavesProvided,
        ));
    }
    let root = if leaves.len() == 1 && siblings.is_empty() {
        leaves[0]
    } else {
        let num_left = compute_num_left_siblings(0);
        let num_right =
            siblings
                .len()
                .checked_sub(num_left)
                .ok_or(FibreProofError::RangeProofInvalid(
                    RangeProofError::MissingProofNode,
                ))?;
        let tree_size = compute_tree_size(num_right, leaves.len() - 1)
            .map_err(FibreProofError::RangeProofInvalid)?;
        let hasher = TmSha2Hasher::new();
        let mut leaf_rest = leaves;
        let mut proof_rest = siblings;
        fold(&hasher, &mut leaf_rest, &mut proof_rest, 0, tree_size, 0)
            .map_err(FibreProofError::RangeProofInvalid)?
    };
    Proof::<TmSha2Hasher> {
        siblings: siblings.to_vec(),
        range: 0..u32::try_from(leaves.len()).unwrap_or(u32::MAX),
    }
    .verify_range(&root, leaves)
    .map_err(FibreProofError::RangeProofInvalid)?;
    Ok(root)
}

fn fold(
    hasher: &TmSha2Hasher,
    leaves: &mut &[[u8; 32]],
    proof: &mut &[[u8; 32]],
    leaves_start_idx: usize,
    subtrie_size: usize,
    offset: usize,
) -> Result<[u8; 32], RangeProofError> {
    let split_point = subtrie_size.next_power_of_two() >> 1;
    let leaves_end_idx = (leaves.len() + leaves_start_idx) - 1;
    let right = if leaves_end_idx >= split_point + offset {
        let right_size = subtrie_size - split_point;
        if right_size == 1 {
            take_last(leaves).ok_or(RangeProofError::MissingLeaf)?
        } else {
            fold(
                hasher,
                leaves,
                proof,
                leaves_start_idx,
                right_size,
                offset + split_point,
            )?
        }
    } else {
        take_last(proof).ok_or(RangeProofError::MissingProofNode)?
    };
    let left = if leaves_start_idx < split_point + offset {
        if split_point == 1 {
            take_last(leaves).ok_or(RangeProofError::MissingLeaf)?
        } else {
            fold(hasher, leaves, proof, leaves_start_idx, split_point, offset)?
        }
    } else {
        take_last(proof).ok_or(RangeProofError::MissingProofNode)?
    };
    Ok(hasher.hash_nodes(&left, &right))
}

fn take_last<T: Copy>(items: &mut &[T]) -> Option<T> {
    let (last, rest) = items.split_last()?;
    *items = rest;
    Some(*last)
}

#[derive(Clone)]
pub struct CelestiaFibreVerifier {
    rollup_batch_namespace: Namespace,
    rollup_proof_namespace: Namespace,
}

impl DaVerifier for CelestiaFibreVerifier {
    type Spec = CelestiaFibreSpec;
    type Error = ValidationError;

    fn new(params: RollupParams) -> Self {
        Self {
            rollup_batch_namespace: params.rollup_batch_namespace,
            rollup_proof_namespace: params.rollup_proof_namespace,
        }
    }

    fn verify_relevant_tx_list(
        &self,
        block_header: &CelestiaHeader,
        relevant_blobs: &RelevantBlobs<FibreBlob>,
        relevant_proofs: RelevantProofs<Vec<FibreBlobProof>, Option<NamespaceBoundaryProof>>,
    ) -> Result<(), Self::Error> {
        block_header.validate_dah()?;
        verify_one(
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
        verify_one(
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

fn verify_one(
    block_header: &CelestiaHeader,
    blobs: &[FibreBlob],
    namespace: Namespace,
    inclusion: Vec<FibreBlobProof>,
    boundary: Option<NamespaceBoundaryProof>,
) -> Result<(), NamespaceValidationError> {
    let namespace_row_roots: Vec<&NamespacedHash> = block_header
        .dah
        .row_roots()
        .iter()
        .filter(|root| root.contains::<NamespacedSha2Hasher<NS_SIZE>>(namespace.into()))
        .collect();
    let pointers = inclusion
        .iter()
        .map(|proof| proof.pointer.clone())
        .collect::<Vec<_>>();
    let mut rows_by_proof = inclusion;
    verify_namespace(
        block_header,
        blobs,
        namespace,
        pointers,
        boundary,
        supported_share,
        |blob, pointer| {
            let idx = rows_by_proof
                .iter()
                .position(|proof| proof.pointer == pointer)
                .expect("rows proof missing for pointer");
            let rows = rows_by_proof.remove(idx).rows;
            authenticate_fibre_blob(
                block_header,
                &namespace_row_roots,
                namespace,
                blob,
                pointer,
                rows,
            )
        },
    )
}

fn authenticate_fibre_blob(
    block_header: &CelestiaHeader,
    namespace_row_roots: &[&NamespacedHash],
    namespace: Namespace,
    blob: &FibreBlob,
    mut blob_row_proof: BlobProof,
    rows: Option<FibreRowsProof>,
) -> Result<usize, NamespaceValidationError> {
    let share_count: usize = blob_row_proof
        .range_proofs
        .iter()
        .map(|p| p.shares.len())
        .sum();
    if share_count != 1 {
        return Err(InvalidRowProof(RowProofError::WrongNumberOfShares {
            expected: 1,
            actual: share_count,
        }));
    }
    let sub_proof = blob_row_proof.range_proofs.remove(0);
    let row_number = block_header.calculate_row_number_for_share(sub_proof.start_share_idx);
    let row_root = namespace_row_roots[row_number];
    let share = &sub_proof.shares[0];
    let pointer =
        parse_v2_pointer(share).map_err(|_| InvalidBlobData(BlobDataError::NonMatchingShare))?;
    let signer = share
        .signer()
        .map(CelestiaAddress)
        .ok_or(InvalidBlobData(BlobDataError::NonMatchingShare))?;
    if signer != blob.sender() {
        return Err(InvalidBlobData(BlobDataError::WrongSender));
    }
    if pointer.commitment != blob.commitment() {
        return Err(InvalidBlobData(BlobDataError::NonMatchingShare));
    }
    let raw_leaves = vec![share.data()];
    sub_proof
        .proof
        .verify_range(row_root, &raw_leaves, namespace.into())
        .map_err(|error| InvalidRowProof(RowProofError::ProofError(ProofError::Invalid(error))))?;
    let rows = rows
        .ok_or_else(RowProofError::missing_proof)
        .map_err(InvalidRowProof)?;
    verify_fibre_prefix(
        &pointer.commitment,
        &rows,
        blob.verified_data(),
        blob.total_len(),
    )
    .map_err(|error| match error {
        FibreProofError::RangeProofInvalid(inner) => {
            InvalidRowProof(RowProofError::ProofError(ProofError::Invalid(inner)))
        }
        FibreProofError::MissingProof => InvalidRowProof(RowProofError::missing_proof()),
        _ => InvalidBlobData(BlobDataError::NonMatchingShare),
    })?;
    let occupied = shares_needed_for_bytes_with_signer(
        share.sequence_length().unwrap_or(0) as usize,
        share.signer().is_some(),
    )
    .max(1);
    Ok(occupied)
}

#[cfg(test)]
mod tests {
    use nmt_rs::simple_merkle::db::MemDb;
    use nmt_rs::simple_merkle::tree::MerkleTree;

    use super::*;

    const DATA_LEN: usize = 0x010203;
    const RLC: [u8; 32] = [9u8; 32];

    fn test_data() -> Vec<u8> {
        (0..DATA_LEN).map(|i| (i * 31 + i / 251) as u8).collect()
    }

    fn matrix(header: [u8; FIBRE_HEADER_LEN], data: &[u8], row_size: usize) -> Vec<Vec<u8>> {
        let mut buf = header.to_vec();
        buf.extend_from_slice(data);
        buf.resize(row_size * FIBRE_K, 0);
        let mut rows: Vec<Vec<u8>> = buf.chunks(row_size).map(|chunk| chunk.to_vec()).collect();
        rows.resize(FIBRE_K + FIBRE_N, vec![0u8; row_size]);
        rows
    }

    fn open(rows: &[Vec<u8>], prefix_rows: usize) -> ([u8; 32], FibreRowsProof) {
        let mut tree =
            MerkleTree::<MemDb<[u8; 32]>, TmSha2Hasher>::with_hasher(TmSha2Hasher::new());
        for row in rows {
            tree.push_raw_leaf(row);
        }
        let row_root = tree.root();
        let proof = tree.build_range_proof(0..prefix_rows);
        let mut concatenated = Vec::new();
        for row in &rows[..prefix_rows] {
            concatenated.extend_from_slice(row);
        }
        let commitment = sha256_pair(&row_root, &RLC);
        (
            commitment,
            FibreRowsProof {
                row_size: rows[0].len() as u32,
                rlc_root: RLC,
                rows: concatenated,
                siblings: proof.siblings,
            },
        )
    }

    fn canonical(data: &[u8], prefix_len: usize) -> ([u8; 32], FibreRowsProof) {
        let row_size = row_size_for(data.len()).unwrap();
        let mut header = [0u8; FIBRE_HEADER_LEN];
        header[1..].copy_from_slice(&(data.len() as u32).to_be_bytes());
        let rows = matrix(header, data, row_size);
        let prefix_rows = rows_needed_for_prefix(prefix_len, row_size);
        open(&rows, prefix_rows)
    }

    #[test]
    fn row_size_matches_fibre_formula() {
        assert_eq!(row_size_for(1), Some(64));
        assert_eq!(row_size_for(DATA_LEN), Some(64));
        assert_eq!(row_size_for(FIBRE_K * 64 - FIBRE_HEADER_LEN), Some(64));
        assert_eq!(row_size_for(FIBRE_K * 64 - FIBRE_HEADER_LEN + 1), Some(128));
        assert_eq!(
            row_size_for(FIBRE_MAX_BLOB_SIZE - FIBRE_HEADER_LEN),
            Some(FIBRE_MAX_BLOB_SIZE / FIBRE_K)
        );
        assert_eq!(
            row_size_for(FIBRE_MAX_BLOB_SIZE - FIBRE_HEADER_LEN + 1),
            None
        );
    }

    #[test]
    fn rows_needed_counts_the_header() {
        assert_eq!(rows_needed_for_prefix(0, 64), 1);
        assert_eq!(rows_needed_for_prefix(59, 64), 1);
        assert_eq!(rows_needed_for_prefix(60, 64), 2);
    }

    #[test]
    fn verify_fibre_prefix_accepts_full_partial_and_zero_reads() {
        let data = test_data();
        for prefix in [0, 1000, DATA_LEN] {
            let (commitment, proof) = canonical(&data, prefix);
            verify_fibre_prefix(&commitment, &proof, &data[..prefix], DATA_LEN).unwrap();
        }
    }

    #[test]
    fn verify_fibre_prefix_rejects_tampered_row_sibling_and_roots() {
        let data = test_data();
        let (commitment, proof) = canonical(&data, 1000);
        let mut tampered = proof.clone();
        tampered.rows[100] ^= 1;
        assert_eq!(
            verify_fibre_prefix(&commitment, &tampered, &data[..1000], DATA_LEN),
            Err(FibreProofError::CommitmentMismatch)
        );
        let mut sibling = proof.clone();
        sibling.siblings[0][0] ^= 1;
        assert_eq!(
            verify_fibre_prefix(&commitment, &sibling, &data[..1000], DATA_LEN),
            Err(FibreProofError::CommitmentMismatch)
        );
        let mut rlc = proof.clone();
        rlc.rlc_root[0] ^= 1;
        assert_eq!(
            verify_fibre_prefix(&commitment, &rlc, &data[..1000], DATA_LEN),
            Err(FibreProofError::CommitmentMismatch)
        );
        let mut wrong = commitment;
        wrong[31] ^= 1;
        assert_eq!(
            verify_fibre_prefix(&wrong, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::CommitmentMismatch)
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_bad_shape_and_header() {
        let data = test_data();
        let (commitment, mut proof) = canonical(&data, 1000);
        proof.row_size = 96;
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::RowSizeInvalid(96))
        );

        let (commitment, mut proof) = canonical(&data, 1000);
        proof.rows.push(0);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::RowSizeInvalid(64))
        );

        let rows = matrix([1, 0, 1, 2, 3], &data, 64);
        let (commitment, proof) = open(&rows, 16);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::HeaderInvalid)
        );

        let rows = matrix([0, 0, 0, 0, 0], &[], 64);
        let (commitment, proof) = open(&rows, 1);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &[], 0),
            Err(FibreProofError::HeaderInvalid)
        );

        let rows = matrix([0, 3, 2, 1, 0], &data, 64);
        let (commitment, proof) = open(&rows, 16);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::DataSizeMismatch {
                expected: DATA_LEN as u64,
                actual: 0x03020100,
            })
        );

        let (commitment, proof) = canonical(&data, 1000);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN + 1),
            Err(FibreProofError::DataSizeMismatch {
                expected: DATA_LEN as u64 + 1,
                actual: DATA_LEN as u64,
            })
        );

        let mut header = [0u8; 5];
        header[1..].copy_from_slice(&(DATA_LEN as u32).to_be_bytes());
        let rows = matrix(header, &data, 128);
        let (commitment, proof) = open(&rows, 8);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::RowSizeInvalid(128))
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_prefix_and_row_count_mismatches() {
        let data = test_data();
        let (commitment, proof) = canonical(&data, 1000);
        let mut prefix = data[..1000].to_vec();
        prefix[999] ^= 1;
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &prefix, DATA_LEN),
            Err(FibreProofError::PrefixMismatch)
        );
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..2000], DATA_LEN),
            Err(FibreProofError::RowCountMismatch {
                expected: 32,
                actual: 16
            })
        );
        let (commitment, proof) = canonical(&data, 2000);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::RowCountMismatch {
                expected: 16,
                actual: 32
            })
        );
        let short = vec![7u8; 10];
        let (commitment, proof) = canonical(&short, 10);
        let mut prefix = short.clone();
        prefix.extend_from_slice(&[0; 4]);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &prefix, short.len()),
            Err(FibreProofError::PrefixMismatch)
        );
    }
}
