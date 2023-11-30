//! Storage and state management interfaces for Sovereign SDK modules.

#![deny(missing_docs)]

pub mod codec;

#[cfg(feature = "native")]
mod prover_storage;

mod witness;
mod zk_storage;

#[cfg(feature = "native")]
pub use prover_storage::ProverStorage;
pub use zk_storage::ZkStorage;

pub mod config;

pub use sov_modules_core::{
    storage, AlignedVec, CacheLog, OrderedReadsAndWrites, Prefix, Storage, StorageInternalCache,
    Witness,
};
use sov_rollup_interface::digest::Digest;

pub use crate::witness::ArrayWitness;

/// A trait specifying the hash function and format of the witness used in
/// merkle proofs for storage access
pub trait MerkleProofSpec {
    /// The structure that accumulates the witness data
    type Witness: Witness + Send + Sync;
    /// The hash function used to compute the merkle root
    type Hasher: Digest<OutputSize = sha2::digest::typenum::U32>;
}

use sha2::Sha256;

/// The default [`MerkleProofSpec`] implementation.
///
/// This type is typically found as a type parameter for [`ProverStorage`].
#[derive(Clone)]
pub struct DefaultStorageSpec;

impl MerkleProofSpec for DefaultStorageSpec {
    type Witness = ArrayWitness;

    type Hasher = Sha256;
}
