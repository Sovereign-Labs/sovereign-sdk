use serde::{Deserialize, Serialize};
use sov_celestia_common::proofs::BlobProof;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FibreBlobProof {
    pub pointer: BlobProof,
    pub rows: Option<FibreRowsProof>,
}

/// Opening of rows `0..m` of a Fibre blob. `row_root` is not stored: the verifier folds it
/// from `rows` and `siblings`, then checks `sha256(row_root ‖ rlc_root)` against the pointer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FibreRowsProof {
    pub row_size: u32,
    pub rlc_root: [u8; 32],
    pub rows: Vec<u8>,
    pub siblings: Vec<[u8; 32]>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum FibreProofError {
    #[error("Share is not a valid Fibre pointer")]
    InvalidPointer,
    #[error("Unsupported Fibre blob version {0}")]
    UnsupportedBlobVersion(u32),
    #[error("Invalid Fibre row size {0}")]
    RowSizeInvalid(u32),
    #[error("Expected {expected} Fibre rows, got {actual}")]
    RowCountMismatch { expected: usize, actual: usize },
    #[error("Invalid Fibre row range proof: {0:?}")]
    RangeProofInvalid(nmt_rs::simple_merkle::error::RangeProofError),
    #[error("Fibre rows do not match the commitment")]
    CommitmentMismatch,
    #[error("Invalid Fibre blob header")]
    HeaderInvalid,
    #[error("Fibre blob claims data size {actual} but {expected} was expected")]
    DataSizeMismatch { expected: u64, actual: u64 },
    #[error("Fibre rows do not contain the claimed data prefix")]
    PrefixMismatch,
    #[error("Fibre rows proof is missing")]
    MissingProof,
}
