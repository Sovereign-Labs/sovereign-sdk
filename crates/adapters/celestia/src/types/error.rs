#[cfg(feature = "native")]
use celestia_types::nmt::Namespace;

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Missing data hash in header")]
    MissingDataHash,

    #[error("Data root hash doesn't match computed one")]
    InvalidDataRoot,

    #[error(transparent)]
    DahValidation(#[from] celestia_types::ValidationError),

    #[error("Namespace validation error: {namespace:?} {error:?}")]
    NamespaceValidationError {
        namespace: NamespaceType,
        error: NamespaceValidationError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum NamespaceValidationError {
    #[error("Invalid proof {0}")]
    InvalidBlobData(BlobDataError),

    #[error("Invalid row proof: {0}")]
    InvalidRowProof(RowProofError),

    #[error("Incomplete namespace: {0}")]
    IncompleteNamespace(IncompleteNamespaceError),
}

#[derive(Debug, thiserror::Error)]
pub enum IncompleteNamespaceError {
    #[error("Boundary proof error: {0:?}")]
    ProofError(ProofError),
    #[error("Missing blobs")]
    MissingBlobs,
}

impl IncompleteNamespaceError {
    pub(crate) fn corrupted_proof() -> Self {
        IncompleteNamespaceError::ProofError(ProofError::Corrupted)
    }
}

#[derive(Debug)]
pub enum NamespaceType {
    Batch,
    Proof,
}

#[derive(Debug, thiserror::Error)]
pub enum BlobDataError {
    #[error("More proofs than blobs")]
    MoreProofsThanBlobs,
    #[error("Unexpected blobs. Namespace should have no blobs for this namespace.")]
    UnexpectedBlobs,
    #[error("Share does not match provided blob with sender")]
    NonMatchingShare,
    #[error("Wrong sender")]
    WrongSender,
}

#[derive(Debug, thiserror::Error)]
pub enum RowProofError {
    #[error("Wrong start share index: expected {expected} expected, got {actual}.")]
    WrongStartShareIndex { expected: usize, actual: usize },
    #[error("Wrong number of shares proven: expected {expected}, actual {actual}")]
    WrongNumberOfShares { expected: usize, actual: usize },
    #[error("Row proof error: {0:?}")]
    ProofError(ProofError),
}

impl RowProofError {
    pub(crate) fn missing_proof() -> Self {
        RowProofError::ProofError(ProofError::Missing)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("Proof is missing")]
    Missing,
    #[error("Proof is corrupted")]
    Corrupted,
    #[error("Invalid NMT proof: {0:?}")]
    Invalid(nmt_rs::simple_merkle::error::RangeProofError),
}

#[cfg(feature = "native")]
#[derive(Debug, thiserror::Error)]
pub enum ExtractionProofError {
    #[error(
        "Supported shares exist in namespace {namespace:?}, but extracted blobs are empty (namespace share count: {share_count})"
    )]
    MissingBlobsForSupportedNamespace {
        namespace: Namespace,
        share_count: usize,
    },
    #[error("Blob range has invalid bounds: start {start}, end {end}")]
    InvalidBlobRange { start: usize, end: usize },
    #[error(
        "Blob range is out of bounds: start {start}, end {end}, namespace shares {namespace_shares}"
    )]
    BlobRangeOutOfBounds {
        start: usize,
        end: usize,
        namespace_shares: usize,
    },
    #[error(
        "Blob coverage exceeds declared range: start {start}, declared_end {declared_end}, required_end {required_end}"
    )]
    BlobRangeCoverageExceeded {
        start: usize,
        declared_end: usize,
        required_end: usize,
    },
    #[error("Missing info byte for share {share_idx}")]
    MissingInfoByte { share_idx: usize },
    #[error("Missing sequence length for share {share_idx}")]
    MissingSequenceLength { share_idx: usize },
    #[error("Invalid tail padding for share {share_idx}")]
    InvalidTailPadding { share_idx: usize },
    #[error("Supported share version encountered in skipped range at share {share_idx}")]
    SupportedShareInSkippedRange { share_idx: usize },
    #[error("Row root out of bounds: row {row_num}, row roots {row_roots_len}")]
    RowRootOutOfBounds {
        row_num: usize,
        row_roots_len: usize,
    },
    #[error("Namespace row out of bounds: row {row_num}, rows {rows_len}")]
    NamespaceRowOutOfBounds { row_num: usize, rows_len: usize },
    #[error(
        "Row share range out of bounds: row {row_num}, start {start}, end {end}, row shares {row_shares_len}"
    )]
    RowShareRangeOutOfBounds {
        row_num: usize,
        start: usize,
        end: usize,
        row_shares_len: usize,
    },
    #[error("Failed to narrow row proof: {0:?}")]
    NarrowRangeProof(nmt_rs::simple_merkle::error::RangeProofError),
    #[error("Invalid proof self-check: {0:?}")]
    ProofSelfCheck(nmt_rs::simple_merkle::error::RangeProofError),
    #[error("Empty proof for blob range {start}..{end}")]
    EmptyBlobProof { start: usize, end: usize },
    #[error("Invalid namespace boundary data: last row proof is of presence, but no shares")]
    InvalidNamespaceBoundaryData,
}
