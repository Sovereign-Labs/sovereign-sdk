//! Verification of Fibre (share version 2) blobs.
//!
//! A Fibre blob is represented on Celestia by a single share holding a pointer: the blob version
//! and a RSEMA1D commitment `sha256(row_root ‖ rlc_root)`. The blob data itself lives off-chain
//! in `FIBRE_K + FIBRE_N` erasure-coded rows. `row_root` is an RFC 6962 Merkle root over the
//! rows, with leaves `sha256(0x00 ‖ row)`, so the commitment binds every row to its index.
//!
//! The verifier only needs integrity of the bytes the rollup consumed. It therefore checks a
//! SHA-256 Merkle range proof for the first rows covering the consumed prefix and does not
//! re-check the RLC / Reed-Solomon relations: those were enforced by validators before they
//! signed the payment promise, and by the chain before it synthesized the pointer share.
//!
//! `K`, `N` and the row alignment are protocol defaults; a governance change requires an adapter
//! release.

use celestia_types::Share;
use nmt_rs::simple_merkle::proof::Proof;
use nmt_rs::simple_merkle::tree::MerkleHash;
use nmt_rs::TmSha2Hasher;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::FibreProofError;

pub(crate) const FIBRE_SHARE_VERSION: u8 = 2;
pub(crate) const FIBRE_BLOB_VERSION: u32 = 0;
/// Number of original rows.
pub(crate) const FIBRE_K: usize = 4096;
/// Number of parity rows.
pub(crate) const FIBRE_N: usize = 12288;
/// Blob header: `[version: u8][data_size: u32 BE]`.
pub(crate) const FIBRE_HEADER_LEN: usize = 5;
pub(crate) const FIBRE_ROW_ALIGN: usize = 64;
/// Maximum blob size including the header.
pub(crate) const FIBRE_MAX_BLOB_SIZE: usize = 1 << 27;
/// Pointer payload: `u32 BE blob_version ‖ 32-byte commitment`.
pub(crate) const FIBRE_POINTER_LEN: u32 = 36;

const _: () = assert!(FIBRE_K.is_power_of_two() && (FIBRE_K + FIBRE_N).is_power_of_two());

/// The content of a Fibre pointer share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FibrePointer {
    pub(crate) blob_version: u32,
    pub(crate) commitment: [u8; 32],
}

pub(crate) fn parse_v2_pointer(share: &Share) -> Result<FibrePointer, FibreProofError> {
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

/// Row size used to encode `data_len` bytes, or `None` if the blob exceeds the protocol maximum.
pub(crate) fn row_size_for(data_len: usize) -> Option<usize> {
    let blob_len = data_len.checked_add(FIBRE_HEADER_LEN)?;
    if blob_len > FIBRE_MAX_BLOB_SIZE {
        return None;
    }
    Some(blob_len.div_ceil(FIBRE_K).next_multiple_of(FIBRE_ROW_ALIGN))
}

/// Number of leading rows that hold the header and the first `prefix_len` data bytes.
/// `row_size` must be non-zero.
pub(crate) fn rows_needed_for_prefix(prefix_len: usize, row_size: usize) -> usize {
    FIBRE_HEADER_LEN
        .saturating_add(prefix_len)
        .div_ceil(row_size)
        .max(1)
}

/// Opening of the first rows of a Fibre blob, enough to authenticate a data prefix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FibreRowsProof {
    pub(crate) row_size: u32,
    pub(crate) row_root: [u8; 32],
    pub(crate) rlc_root: [u8; 32],
    /// Rows `0..m`, concatenated.
    pub(crate) rows: Vec<u8>,
    /// Siblings of the range proof of leaves `0..m` against `row_root`.
    pub(crate) siblings: Vec<[u8; 32]>,
}

fn sha256_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// Checks that `proof` opens rows of the blob committed to by `commitment` and that those rows
/// contain `expected_prefix` as the start of a blob of `expected_total_len` bytes.
pub(crate) fn verify_fibre_prefix(
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
    Proof::<TmSha2Hasher> {
        siblings: proof.siblings.clone(),
        range: 0..actual_rows as u32,
    }
    .verify_range(&proof.row_root, &leaves)
    .map_err(FibreProofError::RangeProofInvalid)?;
    if sha256_pair(&proof.row_root, &proof.rlc_root) != *commitment {
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

/// Everything about an encoded blob that is needed to build [`FibreRowsProof`]s.
#[cfg(feature = "native")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FibreEncoded {
    pub(crate) commitment: [u8; 32],
    pub(crate) row_size: u32,
    pub(crate) row_root: [u8; 32],
    pub(crate) rlc_root: [u8; 32],
    /// `sha256(0x00 ‖ row)` for all `FIBRE_K + FIBRE_N` rows, in index order.
    pub(crate) leaf_hashes: Vec<[u8; 32]>,
}

/// Header followed by `data`, without row padding.
#[cfg(feature = "native")]
fn blob_bytes(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(!data.is_empty(), "Fibre blobs cannot be empty");
    let data_size = u32::try_from(data.len())?;
    let mut blob = Vec::with_capacity(FIBRE_HEADER_LEN + data.len());
    blob.push(0);
    blob.extend_from_slice(&data_size.to_be_bytes());
    blob.extend_from_slice(data);
    Ok(blob)
}

/// Encodes a zero-padded buffer of `FIBRE_K` original rows.
#[cfg(feature = "native")]
fn encode_original_rows(buf: Vec<u8>, row_size: usize) -> anyhow::Result<FibreEncoded> {
    let params = rsema1d::Parameters::new(FIBRE_K, FIBRE_N, row_size)?;
    let rows = rsema1d::RowMatrix::with_shape(buf, FIBRE_K, row_size)?;
    let (extended, _, _) = rsema1d::encode(&rows, &params)?;
    let hasher = TmSha2Hasher::new();
    let leaf_hashes = (0..FIBRE_K + FIBRE_N)
        .map(|index| Ok(hasher.hash_leaf(extended.row(index)?)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(FibreEncoded {
        commitment: extended.commitment(),
        row_size: u32::try_from(row_size)?,
        row_root: extended.row_root(),
        rlc_root: extended.rlc_root(),
        leaf_hashes,
    })
}

/// Re-encodes `data` exactly as a Fibre client does, to recover the row commitment data that
/// validators do not hand out.
#[cfg(feature = "native")]
pub(crate) fn encode_for_proofs(data: &[u8]) -> anyhow::Result<FibreEncoded> {
    let row_size = row_size_for(data.len()).ok_or_else(|| {
        anyhow::anyhow!(
            "Fibre blob of {} bytes exceeds the maximum size",
            data.len()
        )
    })?;
    let mut buf = blob_bytes(data)?;
    buf.resize(row_size * FIBRE_K, 0);
    encode_original_rows(buf, row_size)
}

/// Roots of the subtrees to the right of the first `prefix_rows` leaves, in in-order. `leaves`
/// must have a power-of-two length.
#[cfg(feature = "native")]
fn prefix_siblings(leaves: &[[u8; 32]], prefix_rows: usize, siblings: &mut Vec<[u8; 32]>) {
    if prefix_rows == leaves.len() {
        return;
    }
    let (left, right) = leaves.split_at(leaves.len() / 2);
    if prefix_rows <= left.len() {
        prefix_siblings(left, prefix_rows, siblings);
        siblings.push(subtree_root(right));
    } else {
        prefix_siblings(right, prefix_rows - left.len(), siblings);
    }
}

#[cfg(feature = "native")]
fn subtree_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    match leaves {
        [leaf] => *leaf,
        _ => {
            let (left, right) = leaves.split_at(leaves.len() / 2);
            TmSha2Hasher::new().hash_nodes(&subtree_root(left), &subtree_root(right))
        }
    }
}

/// Opens the leading rows of `encoded` that cover the header and the first `prefix_len` bytes of
/// `data`. `data` must be the blob `encoded` was created from and `prefix_len <= data.len()`.
#[cfg(feature = "native")]
pub(crate) fn build_rows_proof(
    encoded: &FibreEncoded,
    data: &[u8],
    prefix_len: usize,
) -> FibreRowsProof {
    let row_size = encoded.row_size as usize;
    let prefix_rows = rows_needed_for_prefix(prefix_len, row_size);
    let mut rows = blob_bytes(data).expect("bug: Fibre data was already encoded");
    rows.resize(prefix_rows * row_size, 0);
    rows_proof(encoded, rows)
}

#[cfg(feature = "native")]
fn rows_proof(encoded: &FibreEncoded, rows: Vec<u8>) -> FibreRowsProof {
    let row_size = encoded.row_size as usize;
    let prefix_rows = rows.len() / row_size;
    assert!(
        (1..=FIBRE_K).contains(&prefix_rows),
        "bug: Fibre prefix of {prefix_rows} rows is out of range"
    );
    let mut siblings = Vec::new();
    prefix_siblings(&encoded.leaf_hashes, prefix_rows, &mut siblings);
    FibreRowsProof {
        row_size: encoded.row_size,
        row_root: encoded.row_root,
        rlc_root: encoded.rlc_root,
        rows,
        siblings,
    }
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use nmt_rs::simple_merkle::error::RangeProofError;

    use super::*;

    // 0x010203: asymmetric big-endian bytes, larger than 256.
    const DATA_LEN: usize = 0x010203;

    fn test_data() -> Vec<u8> {
        (0..DATA_LEN).map(|i| (i * 31 + i / 251) as u8).collect()
    }

    struct Fixture {
        data: Vec<u8>,
        encoded: FibreEncoded,
    }

    impl Fixture {
        fn new() -> Self {
            let data = test_data();
            let encoded = encode_for_proofs(&data).unwrap();
            Self { data, encoded }
        }

        fn proof(&self, prefix_len: usize) -> FibreRowsProof {
            build_rows_proof(&self.encoded, &self.data, prefix_len)
        }

        fn verify(&self, proof: &FibreRowsProof, prefix_len: usize) -> Result<(), FibreProofError> {
            verify_fibre_prefix(
                &self.encoded.commitment,
                proof,
                &self.data[..prefix_len],
                self.data.len(),
            )
        }
    }

    /// Encodes `buf` (all `FIBRE_K` original rows) and opens its first `prefix_rows` rows.
    fn custom_blob(
        buf: Vec<u8>,
        row_size: usize,
        prefix_rows: usize,
    ) -> ([u8; 32], FibreRowsProof) {
        let rows = buf[..prefix_rows * row_size].to_vec();
        let encoded = encode_original_rows(buf, row_size).unwrap();
        (encoded.commitment, rows_proof(&encoded, rows))
    }

    fn padded_blob(header: [u8; FIBRE_HEADER_LEN], data: &[u8], row_size: usize) -> Vec<u8> {
        let mut buf = header.to_vec();
        buf.extend_from_slice(data);
        buf.resize(row_size * FIBRE_K, 0);
        buf
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
    fn verify_fibre_prefix_accepts_full_read() {
        let fixture = Fixture::new();
        let proof = fixture.proof(DATA_LEN);
        assert_eq!(fixture.verify(&proof, DATA_LEN), Ok(()));
    }

    #[test]
    fn verify_fibre_prefix_accepts_partial_read() {
        let fixture = Fixture::new();
        let proof = fixture.proof(1000);
        assert_eq!(proof.rows.len(), 16 * 64);
        assert_eq!(fixture.verify(&proof, 1000), Ok(()));
    }

    #[test]
    fn verify_fibre_prefix_accepts_zero_read() {
        let fixture = Fixture::new();
        let proof = fixture.proof(0);
        assert_eq!(proof.rows.len(), 64);
        assert_eq!(fixture.verify(&proof, 0), Ok(()));
    }

    #[test]
    fn verify_fibre_prefix_rejects_tampered_row() {
        let fixture = Fixture::new();
        let mut proof = fixture.proof(1000);
        proof.rows[100] ^= 1;
        assert_eq!(
            fixture.verify(&proof, 1000),
            Err(FibreProofError::RangeProofInvalid(
                RangeProofError::InvalidRoot
            ))
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_wrong_rlc_root() {
        let fixture = Fixture::new();
        let mut proof = fixture.proof(1000);
        proof.rlc_root[0] ^= 1;
        assert_eq!(
            fixture.verify(&proof, 1000),
            Err(FibreProofError::CommitmentMismatch)
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_wrong_commitment() {
        let fixture = Fixture::new();
        let proof = fixture.proof(1000);
        let mut commitment = fixture.encoded.commitment;
        commitment[31] ^= 1;
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &fixture.data[..1000], DATA_LEN),
            Err(FibreProofError::CommitmentMismatch)
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_non_canonical_row_size() {
        let data = test_data();
        let buf = padded_blob([0, 0, 1, 2, 3], &data, 128);
        let (commitment, proof) = custom_blob(buf, 128, 8);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::RowSizeInvalid(128))
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_unaligned_row_size() {
        let fixture = Fixture::new();
        let mut proof = fixture.proof(1000);
        proof.row_size = 96;
        assert_eq!(
            fixture.verify(&proof, 1000),
            Err(FibreProofError::RowSizeInvalid(96))
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_partial_trailing_row() {
        let fixture = Fixture::new();
        let mut proof = fixture.proof(1000);
        proof.rows.push(0);
        assert_eq!(
            fixture.verify(&proof, 1000),
            Err(FibreProofError::RowSizeInvalid(64))
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_unknown_header_version() {
        let data = test_data();
        let buf = padded_blob([1, 0, 1, 2, 3], &data, 64);
        let (commitment, proof) = custom_blob(buf, 64, 16);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::HeaderInvalid)
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_empty_blob_header() {
        let buf = padded_blob([0, 0, 0, 0, 0], &[], 64);
        let (commitment, proof) = custom_blob(buf, 64, 1);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &[], 0),
            Err(FibreProofError::HeaderInvalid)
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_data_size_mismatch() {
        let fixture = Fixture::new();
        let proof = fixture.proof(1000);
        assert_eq!(
            verify_fibre_prefix(
                &fixture.encoded.commitment,
                &proof,
                &fixture.data[..1000],
                DATA_LEN + 1
            ),
            Err(FibreProofError::DataSizeMismatch {
                expected: DATA_LEN as u64 + 1,
                actual: DATA_LEN as u64,
            })
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_swapped_data_size_bytes() {
        let data = test_data();
        let buf = padded_blob([0, 3, 2, 1, 0], &data, 64);
        let (commitment, proof) = custom_blob(buf, 64, 16);
        assert_eq!(
            verify_fibre_prefix(&commitment, &proof, &data[..1000], DATA_LEN),
            Err(FibreProofError::DataSizeMismatch {
                expected: DATA_LEN as u64,
                actual: 0x03020100,
            })
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_prefix_mismatch() {
        let fixture = Fixture::new();
        let proof = fixture.proof(1000);
        let mut prefix = fixture.data[..1000].to_vec();
        prefix[999] ^= 1;
        assert_eq!(
            verify_fibre_prefix(&fixture.encoded.commitment, &proof, &prefix, DATA_LEN),
            Err(FibreProofError::PrefixMismatch)
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_prefix_longer_than_blob() {
        let data = vec![7u8; 10];
        let encoded = encode_for_proofs(&data).unwrap();
        let proof = build_rows_proof(&encoded, &data, 10);
        let mut prefix = data.clone();
        prefix.extend_from_slice(&[0; 4]);
        assert_eq!(
            verify_fibre_prefix(&encoded.commitment, &proof, &prefix, data.len()),
            Err(FibreProofError::PrefixMismatch)
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_too_few_rows() {
        let fixture = Fixture::new();
        let proof = fixture.proof(1000);
        assert_eq!(
            fixture.verify(&proof, 2000),
            Err(FibreProofError::RowCountMismatch {
                expected: 32,
                actual: 16
            })
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_too_many_rows() {
        let fixture = Fixture::new();
        let proof = fixture.proof(2000);
        assert_eq!(
            fixture.verify(&proof, 1000),
            Err(FibreProofError::RowCountMismatch {
                expected: 16,
                actual: 32
            })
        );
    }

    #[test]
    fn verify_fibre_prefix_rejects_tampered_siblings() {
        let fixture = Fixture::new();
        let mut proof = fixture.proof(1000);
        proof.siblings[0][0] ^= 1;
        assert_eq!(
            fixture.verify(&proof, 1000),
            Err(FibreProofError::RangeProofInvalid(
                RangeProofError::InvalidRoot
            ))
        );
    }

    #[test]
    fn encode_for_proofs_matches_rsema1d_commitment() {
        let fixture = Fixture::new();
        let encoded = &fixture.encoded;
        assert_eq!(encoded.leaf_hashes.len(), FIBRE_K + FIBRE_N);
        assert_eq!(subtree_root(&encoded.leaf_hashes), encoded.row_root);
        assert_eq!(
            sha256_pair(&encoded.row_root, &encoded.rlc_root),
            encoded.commitment
        );
    }

    #[test]
    fn leaf_hash_openings_verify_with_rsema1d() {
        let fixture = Fixture::new();
        let encoded = &fixture.encoded;
        let blob = blob_bytes(&fixture.data).unwrap();
        let row_size = encoded.row_size as usize;
        let params = rsema1d::Parameters::new(FIBRE_K, FIBRE_N, row_size).unwrap();

        // Bottom-up sibling path of original row 3 in our leaf hashes.
        let index = 3;
        let mut level = encoded.leaf_hashes.clone();
        let mut position = index;
        let mut row_proof = Vec::new();
        while level.len() > 1 {
            row_proof.push(level[position ^ 1]);
            level = level
                .chunks(2)
                .map(|pair| TmSha2Hasher::new().hash_nodes(&pair[0], &pair[1]))
                .collect();
            position /= 2;
        }
        let proof = rsema1d::RowInclusionProof {
            index,
            row: blob[index * row_size..(index + 1) * row_size]
                .to_vec()
                .into(),
            row_proof,
            rlc_root: encoded.rlc_root,
        };
        rsema1d::verify_row_inclusion_proof(&proof, &encoded.commitment, &params).unwrap();
    }

    #[test]
    fn encode_for_proofs_rejects_empty_and_oversized_data() {
        assert!(encode_for_proofs(&[]).is_err());
        assert!(encode_for_proofs(&vec![0; FIBRE_MAX_BLOB_SIZE]).is_err());
    }
}
