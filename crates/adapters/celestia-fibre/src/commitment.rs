//! Verification of a prefix of a Fibre blob's original rows against its commitment.
//!
//! A Fibre commitment is `sha256(row_root || rlc_root)` (rsema1d 1.2.0):
//! - `row_root` is an RFC 6962 Merkle root over all K original and N parity rows;
//! - `rlc_root` is a Merkle root over the random linear combinations (RLC) of the original rows,
//!   with coefficients derived from `row_root`.
//!
//! This module mirrors rsema1d's standalone verification of original rows (row inclusion plus RLC
//! check), extended to a contiguous range `[0, r)` of rows. Original rows hold the blob header and
//! data verbatim, so a reader only pays for the rows covering the bytes it actually reads.
//! Parity rows are never touched: their subtrees are supplied as Merkle siblings.
//!
//! Only blob version 0 is supported, with the default Fibre protocol parameters.

use std::fmt;

use reed_solomon_simd::engine::tables::get_exp_log;
use sha2::{Digest, Sha256};

/// Number of original rows (K) in a version 0 Fibre blob.
const ORIGINAL_ROWS: usize = 4096;
/// Number of parity rows (N) in a version 0 Fibre blob.
const PARITY_ROWS: usize = 12288;
/// Size of the header (version byte and big-endian `u32` data size) prepended to blob data.
const BLOB_HEADER_SIZE: usize = 5;
/// Maximum size of blob data, excluding the header.
pub(crate) const MAX_DATA_SIZE: usize = MAX_BLOB_SIZE - BLOB_HEADER_SIZE;

const BLOB_VERSION: u8 = 0;
const MAX_BLOB_SIZE: usize = 1 << 27;
/// Rows are split into 64-byte chunks of 32 interleaved GF(2^16) symbols.
const CHUNK_SIZE: usize = 64;
const SYMBOLS_PER_CHUNK: usize = CHUNK_SIZE / 2;
/// Row size of a maximum-size blob, see `BlobConfig::max_row_size` in `celestia-fibre`.
const MAX_ROW_SIZE: usize = MAX_BLOB_SIZE / ORIGINAL_ROWS;
/// The row tree has `(K.next_power_of_two() + N).next_power_of_two()` leaves.
const ROW_TREE_DEPTH: u32 = 14;
/// The RLC tree has `K.next_power_of_two()` leaves.
const RLC_TREE_DEPTH: u32 = 12;
/// GF(2^16) has 65535 non-zero elements, so logarithms are `0..=65534`.
const GF_MODULUS: u32 = 65535;
/// Log-table sentinel for a coefficient limb that is zero and has no logarithm.
const ZERO_LIMB: u16 = u16::MAX;

// Original rows occupy tree positions `[0, K)` only because K is a power of two.
const _: () = assert!(ORIGINAL_ROWS == 1 << RLC_TREE_DEPTH);
const _: () = assert!(ORIGINAL_ROWS + PARITY_ROWS == 1 << ROW_TREE_DEPTH);
const _: () = assert!(MAX_ROW_SIZE.is_multiple_of(CHUNK_SIZE));

/// Original rows `[0, r)` of a Fibre blob, with what is needed to check them against the
/// blob commitment.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
)]
pub struct FibreRowsProof {
    /// Size of every row in bytes. Bound by the commitment through the leaves and RLC coefficients.
    pub row_size: u32,
    /// Concatenated original rows `[0, r)`, `r >= 1`. Row 0 starts with the blob header.
    pub rows: Vec<u8>,
    /// Row tree siblings to the right of the range, bottom-up: one for each level at which the
    /// range covers an odd number of nodes.
    pub row_siblings: Vec<[u8; 32]>,
    /// RLC tree siblings to the right of the range, in the same order as `row_siblings`.
    pub rlc_siblings: Vec<[u8; 32]>,
}

/// Blob data authenticated by [`verify_rows_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFibreData<'a> {
    /// Size of the whole blob data, taken from the authenticated header.
    pub data_size: u32,
    /// The data prefix covered by the proven rows: the first `min(data_size, r * row_size - 5)`
    /// bytes of blob data.
    pub data: &'a [u8],
}

/// Reasons a [`FibreRowsProof`] is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FibreProofError {
    /// Row size is zero, not a multiple of 64 or above the protocol maximum.
    InvalidRowSize(u32),
    /// Rows do not form between 1 and K whole rows.
    InvalidRowCount {
        /// Length of [`FibreRowsProof::rows`].
        rows_len: usize,
        /// Claimed row size.
        row_size: u32,
    },
    /// Header version is not 0.
    UnsupportedBlobVersion(u8),
    /// Header declares empty data.
    ZeroDataSize,
    /// Header declares more data than a Fibre blob can carry.
    DataSizeTooLarge(u32),
    /// Header declares more data than K rows of this size can hold.
    DataSizeExceedsRows {
        /// Data size from the header.
        data_size: u32,
        /// Claimed row size.
        row_size: u32,
    },
    /// Wrong number of Merkle siblings for the proven range.
    InvalidSiblingCount,
    /// Rows and siblings do not hash to the expected commitment.
    CommitmentMismatch,
}

impl fmt::Display for FibreProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRowSize(row_size) => write!(f, "invalid Fibre row size {row_size}"),
            Self::InvalidRowCount { rows_len, row_size } => write!(
                f,
                "{rows_len} bytes are not between 1 and {ORIGINAL_ROWS} rows of {row_size} bytes"
            ),
            Self::UnsupportedBlobVersion(version) => {
                write!(f, "unsupported Fibre blob version {version}")
            }
            Self::ZeroDataSize => write!(f, "Fibre blob header declares zero data size"),
            Self::DataSizeTooLarge(data_size) => write!(
                f,
                "Fibre blob data size {data_size} exceeds maximum {MAX_DATA_SIZE}"
            ),
            Self::DataSizeExceedsRows {
                data_size,
                row_size,
            } => write!(
                f,
                "Fibre blob data size {data_size} does not fit into {ORIGINAL_ROWS} rows of {row_size} bytes"
            ),
            Self::InvalidSiblingCount => write!(f, "wrong number of Merkle siblings"),
            Self::CommitmentMismatch => write!(f, "Fibre rows do not match the blob commitment"),
        }
    }
}

impl std::error::Error for FibreProofError {}

/// Checks that `proof` holds the first rows of the Fibre blob with `commitment`, and returns the
/// blob data they cover.
///
/// Accepts exactly the rows a Fibre download verifies and decodes, including non-zero padding and
/// row sizes larger than the minimal one for the data size.
pub fn verify_rows_prefix<'a>(
    commitment: &[u8; 32],
    proof: &'a FibreRowsProof,
) -> Result<VerifiedFibreData<'a>, FibreProofError> {
    let row_size = usize::try_from(proof.row_size)
        .ok()
        .filter(|size| *size != 0 && size.is_multiple_of(CHUNK_SIZE) && *size <= MAX_ROW_SIZE)
        .ok_or(FibreProofError::InvalidRowSize(proof.row_size))?;
    let row_count = proof.rows.len() / row_size;
    if !proof.rows.len().is_multiple_of(row_size) || !(1..=ORIGINAL_ROWS).contains(&row_count) {
        return Err(FibreProofError::InvalidRowCount {
            rows_len: proof.rows.len(),
            row_size: proof.row_size,
        });
    }

    // Cheap header checks first: they reject before any hashing.
    let data_size = parse_header(&proof.rows, proof.row_size)?;

    let row_leaves = proof.rows.chunks_exact(row_size).map(hash_leaf).collect();
    let row_root = fold_prefix(row_leaves, &proof.row_siblings, ROW_TREE_DEPTH)?;

    let mut coefficients = RlcCoefficients::new(&row_root, row_size);
    let rlc_leaves = proof
        .rows
        .chunks_exact(row_size)
        .map(|row| hash_leaf(&coefficients.rlc(row)))
        .collect();
    let rlc_root = fold_prefix(rlc_leaves, &proof.rlc_siblings, RLC_TREE_DEPTH)?;

    if sha256_concat(&[&row_root, &rlc_root]) != *commitment {
        return Err(FibreProofError::CommitmentMismatch);
    }

    let data = &proof.rows[BLOB_HEADER_SIZE..];
    // `data_size` is at most `K * row_size - 5`, so it fits into `usize`.
    let data = &data[..data.len().min(data_size as usize)];
    Ok(VerifiedFibreData { data_size, data })
}

/// Builds the [`FibreRowsProof`] for the rows covering the blob header and the first `read_len`
/// bytes of `data`, by re-encoding `data` the way the Fibre client does on upload.
///
/// Fails if the re-encoded blob doesn't match `commitment`, meaning the uploader didn't use the
/// client's encoding (zero padding, minimal row size). Such rows can't be recovered from `data`.
#[cfg(feature = "native")]
pub(crate) fn prove_rows_prefix(
    commitment: &[u8; 32],
    data: &[u8],
    read_len: usize,
) -> anyhow::Result<FibreRowsProof> {
    let config = celestia_client::api::fibre::BlobConfig::for_version(BLOB_VERSION)?;
    let blob = celestia_client::api::fibre::EncodedFibreBlob::new(data, config)?;
    anyhow::ensure!(
        blob.id().commitment() == *commitment,
        "Fibre blob {} was not encoded like the Fibre client does, its rows can't be rebuilt",
        hex::encode(commitment)
    );

    let row_size = blob.row_size();
    let row_count = (BLOB_HEADER_SIZE + read_len.min(data.len()))
        .div_ceil(row_size)
        .clamp(1, ORIGINAL_ROWS);
    let mut rows = Vec::with_capacity(row_count * row_size);
    for index in 0..row_count {
        rows.extend_from_slice(&blob.row(index)?.row);
    }

    // The right neighbours of the range are the siblings on the path of its last row at the
    // levels where that path goes through a left child.
    let last_row = row_count - 1;
    let row_siblings = blob
        .row(last_row)?
        .row_proof
        .into_iter()
        .enumerate()
        .filter(|(level, _)| (last_row >> level).is_multiple_of(2))
        .map(|(_, sibling)| sibling)
        .collect();
    let rlc_leaves = blob
        .rlc_coeffs()
        .iter()
        .map(|rlc| hash_leaf(&rlc.to_bytes()))
        .collect();

    Ok(FibreRowsProof {
        row_size: u32::try_from(row_size)?,
        rows,
        row_siblings,
        rlc_siblings: prefix_siblings(rlc_leaves, row_count),
    })
}

/// Right-boundary siblings of the range `[0, count)` in the perfect tree over the `level` leaves,
/// in the order [`fold_prefix`] consumes them.
#[cfg(feature = "native")]
fn prefix_siblings(mut level: Vec<[u8; 32]>, mut count: usize) -> Vec<[u8; 32]> {
    let mut siblings = Vec::new();
    while level.len() > 1 {
        if !count.is_multiple_of(2) {
            siblings.push(level[count]);
        }
        level = level
            .chunks_exact(2)
            .map(|pair| hash_internal(&pair[0], &pair[1]))
            .collect();
        count = count.div_ceil(2);
    }
    siblings
}

/// Mirrors `blob_header::decode` from `celestia-fibre`. `rows` holds at least one row of 64+ bytes.
fn parse_header(rows: &[u8], row_size: u32) -> Result<u32, FibreProofError> {
    if rows[0] != BLOB_VERSION {
        return Err(FibreProofError::UnsupportedBlobVersion(rows[0]));
    }
    let data_size = u32::from_be_bytes([rows[1], rows[2], rows[3], rows[4]]);
    if data_size == 0 {
        return Err(FibreProofError::ZeroDataSize);
    }
    if data_size as usize > MAX_DATA_SIZE {
        return Err(FibreProofError::DataSizeTooLarge(data_size));
    }
    // Both factors are bounded by the checks above: at most 4096 * 32768 bytes.
    let capacity = ORIGINAL_ROWS * row_size as usize - BLOB_HEADER_SIZE;
    if data_size as usize > capacity {
        return Err(FibreProofError::DataSizeExceedsRows {
            data_size,
            row_size,
        });
    }
    Ok(data_size)
}

/// Folds the leaves at positions `[0, leaves.len())` of a perfect tree with `2^depth` leaves into
/// its root, taking the right neighbour of the range from `siblings` whenever the range covers an
/// odd number of nodes at a level.
fn fold_prefix(
    mut level: Vec<[u8; 32]>,
    siblings: &[[u8; 32]],
    depth: u32,
) -> Result<[u8; 32], FibreProofError> {
    let mut siblings = siblings.iter();
    for _ in 0..depth {
        if !level.len().is_multiple_of(2) {
            let sibling = siblings
                .next()
                .ok_or(FibreProofError::InvalidSiblingCount)?;
            level.push(*sibling);
        }
        level = level
            .chunks_exact(2)
            .map(|pair| hash_internal(&pair[0], &pair[1]))
            .collect();
    }
    if siblings.next().is_some() {
        return Err(FibreProofError::InvalidSiblingCount);
    }
    match level.as_slice() {
        [root] => Ok(*root),
        // Unreachable for 1..=2^depth leaves, which callers guarantee.
        _ => Err(FibreProofError::InvalidSiblingCount),
    }
}

/// RLC coefficients of one blob, derived lazily: zero symbols don't need their coefficient,
/// so zero padding costs no hashing.
struct RlcCoefficients {
    seed: [u8; 32],
    /// `log(limb)` for each limb of each derived coefficient, [`ZERO_LIMB`] for zero limbs.
    limb_logs: Vec<Option<[u16; 8]>>,
}

impl RlcCoefficients {
    /// Mirrors `derive_coefficients` from rsema1d: the seed binds the row root and parameters.
    fn new(row_root: &[u8; 32], row_size: usize) -> Self {
        let mut params = [0u8; 12];
        // Values are bounded by the protocol constants and fit into `u32`.
        params[0..4].copy_from_slice(&(ORIGINAL_ROWS as u32).to_le_bytes());
        params[4..8].copy_from_slice(&(PARITY_ROWS as u32).to_le_bytes());
        params[8..12].copy_from_slice(&(row_size as u32).to_le_bytes());
        Self {
            seed: sha256_concat(&[row_root, &params]),
            limb_logs: vec![None; row_size / 2],
        }
    }

    fn limb_logs(&mut self, symbol_index: usize) -> [u16; 8] {
        let seed = &self.seed;
        *self.limb_logs[symbol_index].get_or_insert_with(|| {
            // Symbol indices are below `MAX_ROW_SIZE / 2` and fit into `u32`.
            let hash = sha256_concat(&[seed, &(symbol_index as u32).to_le_bytes()]);
            let log = &get_exp_log().log;
            let mut logs = [ZERO_LIMB; 8];
            for (i, limb_log) in logs.iter_mut().enumerate() {
                // `hash_to_gf128`: XOR the two halves of the hash.
                let limb = u16::from_le_bytes([hash[2 * i], hash[2 * i + 1]])
                    ^ u16::from_le_bytes([hash[16 + 2 * i], hash[16 + 2 * i + 1]]);
                if limb != 0 {
                    *limb_log = log[limb as usize];
                }
            }
            logs
        })
    }

    /// RLC of `row` as 16 little-endian bytes: the GF(2^128) sum of `symbol * coefficient` over
    /// the row's symbols, where GF(2^128) is 8 independent GF(2^16) limbs.
    fn rlc(&mut self, row: &[u8]) -> [u8; 16] {
        let exp_log = get_exp_log();
        let mut acc = [0u16; 8];
        for (chunk_index, chunk) in row.chunks_exact(CHUNK_SIZE).enumerate() {
            for j in 0..SYMBOLS_PER_CHUNK {
                // Leopard interleaved layout: low bytes first, then high bytes.
                let symbol = u16::from_le_bytes([chunk[j], chunk[SYMBOLS_PER_CHUNK + j]]);
                if symbol == 0 {
                    continue;
                }
                let log_symbol = exp_log.log[symbol as usize];
                let limb_logs = self.limb_logs(chunk_index * SYMBOLS_PER_CHUNK + j);
                for (acc, limb_log) in acc.iter_mut().zip(limb_logs) {
                    if limb_log != ZERO_LIMB {
                        *acc ^= exp_log.exp[add_mod(log_symbol, limb_log) as usize];
                    }
                }
            }
        }

        let mut bytes = [0u8; 16];
        for (dst, limb) in bytes.chunks_exact_mut(2).zip(acc) {
            dst.copy_from_slice(&limb.to_le_bytes());
        }
        bytes
    }
}

/// `(a + b) mod 65535` for two GF(2^16) logarithms.
fn add_mod(a: u16, b: u16) -> u16 {
    let sum = u32::from(a) + u32::from(b);
    // The result is below 65535 because both inputs are.
    (if sum >= GF_MODULUS {
        sum - GF_MODULUS
    } else {
        sum
    }) as u16
}

fn sha256_concat(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// RFC 6962 leaf hash.
fn hash_leaf(data: &[u8]) -> [u8; 32] {
    sha256_concat(&[&[0x00], data])
}

/// RFC 6962 internal node hash.
fn hash_internal(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    sha256_concat(&[&[0x01], left, right])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blob encoded by the Fibre client, used as the source of truth.
    struct EncodedBlob {
        commitment: [u8; 32],
        row_size: usize,
        /// All K + N rows.
        rows: Vec<Vec<u8>>,
        rlcs: Vec<[u8; 16]>,
    }

    impl EncodedBlob {
        fn new(data: &[u8]) -> Self {
            let config =
                celestia_client::api::fibre::BlobConfig::for_version(BLOB_VERSION).unwrap();
            let blob = celestia_client::api::fibre::EncodedFibreBlob::new(data, config).unwrap();
            let rows = (0..ORIGINAL_ROWS + PARITY_ROWS)
                .map(|index| blob.row(index).unwrap().row.to_vec())
                .collect();
            Self {
                commitment: blob.id().commitment(),
                row_size: blob.row_size(),
                rows,
                rlcs: blob.rlc_coeffs().iter().map(|rlc| rlc.to_bytes()).collect(),
            }
        }

        /// Builds a proof for rows `[0, row_count)`.
        fn proof(&self, row_count: usize) -> FibreRowsProof {
            let row_leaves = self.rows.iter().map(|row| hash_leaf(row)).collect();
            let rlc_leaves = self.rlcs.iter().map(|rlc| hash_leaf(rlc)).collect();
            FibreRowsProof {
                row_size: self.row_size as u32,
                rows: self.rows[..row_count].concat(),
                row_siblings: prefix_siblings(row_leaves, row_count),
                rlc_siblings: prefix_siblings(rlc_leaves, row_count),
            }
        }
    }

    /// Asymmetric, mostly non-zero bytes.
    fn test_data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 + i / 251 + 1) as u8).collect()
    }

    fn assert_prefix_verifies(data_len: usize, row_counts: &[usize]) {
        let data = test_data(data_len);
        let blob = EncodedBlob::new(&data);
        for &row_count in row_counts {
            let proof = blob.proof(row_count);
            let verified = verify_rows_prefix(&blob.commitment, &proof).unwrap();
            let covered = (row_count * blob.row_size - BLOB_HEADER_SIZE).min(data_len);
            assert_eq!(
                verified,
                VerifiedFibreData {
                    data_size: data_len as u32,
                    data: &data[..covered],
                },
                "data_len {data_len}, row_count {row_count}"
            );
        }
    }

    #[test]
    fn verifies_prefixes_of_tiny_blob() {
        assert_prefix_verifies(1, &[1, 2, 7, ORIGINAL_ROWS]);
    }

    #[test]
    fn verifies_prefixes_of_blob_filling_first_row() {
        assert_prefix_verifies(59, &[1, 3, ORIGINAL_ROWS]);
    }

    #[test]
    fn verifies_prefixes_of_blob_filling_all_rows_of_minimal_size() {
        assert_prefix_verifies(
            ORIGINAL_ROWS * 64 - BLOB_HEADER_SIZE,
            &[1, 1000, ORIGINAL_ROWS],
        );
    }

    #[test]
    fn verifies_prefixes_of_blob_with_larger_rows() {
        assert_prefix_verifies(
            300_000,
            &[1, 2, 1234, 2343, ORIGINAL_ROWS - 1, ORIGINAL_ROWS],
        );
    }

    #[test]
    fn verifies_non_zero_padding() {
        // Upload clients pad with zeros, but downloads accept any padding the commitment binds.
        let mut rows = vec![vec![0u8; 64]; ORIGINAL_ROWS];
        rows[0][..BLOB_HEADER_SIZE].copy_from_slice(&[0, 0, 0, 0x01, 0x02]);
        rows[0][BLOB_HEADER_SIZE..].fill(0xAB);
        rows[1].fill(0xCD);
        let row_leaves: Vec<_> = rows
            .iter()
            .map(|row| hash_leaf(row))
            .chain(std::iter::repeat_n(hash_leaf(&[0u8; 64]), PARITY_ROWS))
            .collect();
        let mut coefficients = RlcCoefficients::new(&root(row_leaves.clone()), 64);
        let rlc_leaves: Vec<_> = rows
            .iter()
            .map(|row| hash_leaf(&coefficients.rlc(row)))
            .collect();
        let commitment = sha256_concat(&[&root(row_leaves.clone()), &root(rlc_leaves.clone())]);
        let proof = FibreRowsProof {
            row_size: 64,
            rows: rows[..2].concat(),
            row_siblings: prefix_siblings(row_leaves, 2),
            rlc_siblings: prefix_siblings(rlc_leaves, 2),
        };

        let verified = verify_rows_prefix(&commitment, &proof).unwrap();

        assert_eq!(verified.data_size, 0x0102);
        assert_eq!(verified.data, &proof.rows[BLOB_HEADER_SIZE..]);
    }

    fn root(mut level: Vec<[u8; 32]>) -> [u8; 32] {
        while level.len() > 1 {
            level = level
                .chunks_exact(2)
                .map(|pair| hash_internal(&pair[0], &pair[1]))
                .collect();
        }
        level[0]
    }

    fn valid_blob() -> EncodedBlob {
        EncodedBlob::new(&test_data(300_000))
    }

    #[test]
    fn rejects_modified_row_byte() {
        let blob = valid_blob();
        let mut proof = blob.proof(3);
        proof.rows[200] ^= 0x10;

        assert_eq!(
            verify_rows_prefix(&blob.commitment, &proof),
            Err(FibreProofError::CommitmentMismatch)
        );
    }

    #[test]
    fn rejects_modified_row_sibling() {
        let blob = valid_blob();
        let mut proof = blob.proof(3);
        proof.row_siblings[1][0] ^= 0x10;

        assert_eq!(
            verify_rows_prefix(&blob.commitment, &proof),
            Err(FibreProofError::CommitmentMismatch)
        );
    }

    #[test]
    fn rejects_modified_rlc_sibling() {
        let blob = valid_blob();
        let mut proof = blob.proof(3);
        proof.rlc_siblings[0][0] ^= 0x10;

        assert_eq!(
            verify_rows_prefix(&blob.commitment, &proof),
            Err(FibreProofError::CommitmentMismatch)
        );
    }

    #[test]
    fn rejects_other_commitment() {
        let blob = valid_blob();
        let proof = blob.proof(3);
        let mut commitment = blob.commitment;
        commitment[31] ^= 0x01;

        assert_eq!(
            verify_rows_prefix(&commitment, &proof),
            Err(FibreProofError::CommitmentMismatch)
        );
    }

    #[test]
    fn rejects_extra_sibling() {
        let blob = valid_blob();
        let mut proof = blob.proof(3);
        proof.row_siblings.push([0x12; 32]);

        assert_eq!(
            verify_rows_prefix(&blob.commitment, &proof),
            Err(FibreProofError::InvalidSiblingCount)
        );
    }

    #[test]
    fn rejects_missing_sibling() {
        let blob = valid_blob();
        let mut proof = blob.proof(3);
        proof.rlc_siblings.pop();

        assert_eq!(
            verify_rows_prefix(&blob.commitment, &proof),
            Err(FibreProofError::InvalidSiblingCount)
        );
    }

    #[test]
    fn rejects_row_size_not_multiple_of_chunk() {
        let proof = FibreRowsProof {
            row_size: 96,
            rows: vec![0; 96],
            row_siblings: vec![],
            rlc_siblings: vec![],
        };

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::InvalidRowSize(96))
        );
    }

    #[test]
    fn rejects_row_size_above_maximum() {
        let row_size = (MAX_ROW_SIZE + CHUNK_SIZE) as u32;
        let proof = FibreRowsProof {
            row_size,
            rows: vec![0; row_size as usize],
            row_siblings: vec![],
            rlc_siblings: vec![],
        };

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::InvalidRowSize(row_size))
        );
    }

    #[test]
    fn rejects_empty_rows() {
        let proof = FibreRowsProof {
            row_size: 64,
            rows: vec![],
            row_siblings: vec![],
            rlc_siblings: vec![],
        };

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::InvalidRowCount {
                rows_len: 0,
                row_size: 64
            })
        );
    }

    #[test]
    fn rejects_partial_row() {
        let blob = valid_blob();
        let mut proof = blob.proof(2);
        proof.rows.pop();

        assert_eq!(
            verify_rows_prefix(&blob.commitment, &proof),
            Err(FibreProofError::InvalidRowCount {
                rows_len: 2 * blob.row_size - 1,
                row_size: blob.row_size as u32
            })
        );
    }

    #[test]
    fn rejects_more_than_original_rows() {
        let proof = FibreRowsProof {
            row_size: 64,
            rows: vec![0; 64 * (ORIGINAL_ROWS + 1)],
            row_siblings: vec![],
            rlc_siblings: vec![],
        };

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::InvalidRowCount {
                rows_len: 64 * (ORIGINAL_ROWS + 1),
                row_size: 64
            })
        );
    }

    fn proof_with_header(header: [u8; BLOB_HEADER_SIZE], row_size: u32) -> FibreRowsProof {
        let mut rows = vec![0x5A; row_size as usize];
        rows[..BLOB_HEADER_SIZE].copy_from_slice(&header);
        FibreRowsProof {
            row_size,
            rows,
            row_siblings: vec![],
            rlc_siblings: vec![],
        }
    }

    #[test]
    fn rejects_unsupported_blob_version() {
        let proof = proof_with_header([1, 0, 0, 0x01, 0x02], 64);

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::UnsupportedBlobVersion(1))
        );
    }

    #[test]
    fn rejects_zero_data_size() {
        let proof = proof_with_header([0, 0, 0, 0, 0], 64);

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::ZeroDataSize)
        );
    }

    #[test]
    fn rejects_data_size_above_maximum() {
        let data_size = MAX_DATA_SIZE as u32 + 1;
        let mut header = [0; BLOB_HEADER_SIZE];
        header[1..].copy_from_slice(&data_size.to_be_bytes());
        let proof = proof_with_header(header, MAX_ROW_SIZE as u32);

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::DataSizeTooLarge(data_size))
        );
    }

    #[test]
    fn rejects_data_size_exceeding_rows() {
        let data_size = (ORIGINAL_ROWS * 64 - BLOB_HEADER_SIZE + 1) as u32;
        let mut header = [0; BLOB_HEADER_SIZE];
        header[1..].copy_from_slice(&data_size.to_be_bytes());
        let proof = proof_with_header(header, 64);

        assert_eq!(
            verify_rows_prefix(&[0; 32], &proof),
            Err(FibreProofError::DataSizeExceedsRows {
                data_size,
                row_size: 64
            })
        );
    }

    /// 300 KB of data gives 128-byte rows: row 0 holds the header and 123 data bytes.
    const PROVEN_DATA_LEN: usize = 300_000;
    const PROVEN_ROW_SIZE: usize = 128;

    fn prove(read_len: usize) -> (Vec<u8>, FibreRowsProof, [u8; 32]) {
        let data = test_data(PROVEN_DATA_LEN);
        let commitment = EncodedBlob::new(&data).commitment;
        let proof = prove_rows_prefix(&commitment, &data, read_len).unwrap();
        (data, proof, commitment)
    }

    #[test]
    fn built_proof_covers_read_bytes() {
        for read_len in [0, 1, 123, 124, 150_001, PROVEN_DATA_LEN] {
            let (data, proof, commitment) = prove(read_len);

            let verified = verify_rows_prefix(&commitment, &proof).unwrap();

            assert_eq!(verified.data_size, PROVEN_DATA_LEN as u32);
            assert_eq!(
                verified.data,
                &data[..verified.data.len()],
                "read_len {read_len}"
            );
            assert!(
                verified.data.len() >= read_len,
                "proof for read_len {read_len} covers only {} bytes",
                verified.data.len()
            );
        }
    }

    #[test]
    fn built_proof_has_minimal_rows() {
        for (read_len, row_count) in [
            (0, 1),
            (123, 1),
            (124, 2),
            (150_001, 1172),
            (PROVEN_DATA_LEN, 2344),
        ] {
            let (_, proof, _) = prove(read_len);

            assert_eq!(
                proof.rows.len(),
                row_count * PROVEN_ROW_SIZE,
                "read_len {read_len}"
            );
        }
    }

    #[test]
    fn built_proof_rejects_other_commitment() {
        let data = test_data(PROVEN_DATA_LEN);
        let mut commitment = EncodedBlob::new(&data).commitment;
        commitment[0] ^= 0x01;

        let error = prove_rows_prefix(&commitment, &data, 0).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("was not encoded like the Fibre client"),
            "{error}"
        );
    }
}
