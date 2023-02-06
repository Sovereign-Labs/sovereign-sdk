use thiserror::Error;

use crate::{
    hash::{HashOutput, HashValueParseError},
    KeyHash, ValueHash,
};

#[derive(Debug, Error)]
pub enum CodecError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("nibble path length must not exceed {max:}, but was {got:}")]
    NibblePathTooLong { max: usize, got: usize },
    #[error("encoded num_nibbles {found:?} does not match expected length {expected:}")]
    InvalidNibblePathLength { expected: usize, found: Vec<u8> },
    #[error("invalid nibble path padding. expected 0, got: {got:}")]
    InvalidNibblePathPadding { got: u8 },
    #[error("not enough data to deserialize type: {desired_type:}. needed {needed:}, found {remaining:}")]
    DataTooShort {
        remaining: usize,
        desired_type: &'static str,
        needed: usize,
    },

    #[error(transparent)]
    NodeDecodeError(#[from] NodeDecodeError),
    #[error(transparent)]
    HashValueParseError(#[from] HashValueParseError),
    #[error(transparent)]
    InternalNodeConstructionError(#[from] InternalNodeConstructionError),
    #[error("Unable to decode key of type {key_type:}: err {err:} ")]
    // TODO(preston-evans98): consider making this a generic to avoid conversion to string
    KeyDecodeError { key_type: &'static str, err: String },
    #[error("Unable to fetch node with key {key:}: err {err:} ")]
    NodeFetchError {
        // TODO(preston-evans98): consider making this a generic to avoid conversion to string
        key: String,
        err: String,
    },
}

#[derive(Debug, Error)]
pub enum InternalNodeConstructionError {
    #[error("at least one child must be provided. found none")]
    NoChildrenProvided,
    #[error("if only one child is provided, it must not be a leaf")]
    OnlyChildIsLeaf,
}

#[derive(Debug, Error)]
pub enum ProofError<const N: usize> {
    #[error("SMT proof has {got:} siblings, but no more than {:} are allowed", HashOutput::<N>::LENGTH_IN_BITS)]
    TooManySiblings { got: usize },
    #[error("Keys do not match. Key in proof: {got:}. Expected key: {expected:}")]
    KeyMismatch {
        expected: KeyHash<N>,
        got: KeyHash<N>,
    },

    #[error("value hashes do not match for key {key:}. value hash in proof: {got:}. Expected value hash: {expected:}")]
    ValueMismatch {
        key: KeyHash<N>,
        expected: ValueHash<N>,
        got: ValueHash<N>,
    },
    #[error("Expected inclusion proof, value hash: {value_hash:}. Found non-inclusion proof.")]
    ExpectedInclusionProof { value_hash: ValueHash<N> },
    #[error("Expected non-inclusion proof, but key exists in proof. Key: {leaf_key:}")]
    ExpectedNonInclusionProof { leaf_key: KeyHash<N> },
    #[error("Invalid non-inclusion proof. Inserting key {key_to_verify:} would not yield a subtree with only a single element. Key in proof {key_in_proof:}")]
    InvalidNonInclusionProof {
        key_in_proof: KeyHash<N>,
        key_to_verify: KeyHash<N>,
    },
    #[error(
        "The proof was well-formed but yielded the wrong root. Expected {expected:}, got {got:}"
    )]
    IncorrectRoot {
        expected: HashOutput<N>,
        got: HashOutput<N>,
    },

    #[error("Not enough left siblings were provided. Needed {needed:} siblings, got {got:?}")]
    MissingLeftSibling {
        needed: usize,
        got: Vec<HashOutput<N>>,
    },

    #[error("Not enough right siblings were provided. Needed {needed:} siblings, got {got:?}")]
    MissingRightSibling {
        needed: usize,
        got: Vec<HashOutput<N>>,
    },
}

/// Error thrown when a [`Node`] fails to be deserialized out of a byte sequence stored in physical
/// storage, via [`Node::decode`].
#[derive(Debug, Error, Eq, PartialEq)]
pub enum NodeDecodeError {
    /// Input is empty.
    #[error("Missing tag due to empty input")]
    EmptyInput,

    /// The first byte of the input is not a known tag representing one of the variants.
    #[error("lead tag byte is unknown: {}", unknown_tag)]
    UnknownTag { unknown_tag: u8 },

    /// No children found in internal node
    #[error("No children found in internal node")]
    NoChildren,

    /// Extra leaf bits set
    #[error(
        "Non-existent leaf bits set, existing: {}, leaves: {}",
        existing,
        leaves
    )]
    ExtraLeaves { existing: u16, leaves: u16 },
}
