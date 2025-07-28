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
