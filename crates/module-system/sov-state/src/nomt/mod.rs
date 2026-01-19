//! Storage implementation based on [Nearly Optimal Merkle Tree(NOMT)](https://github.com/thrumdev/nomt/) implementation.

use borsh::{BorshDeserialize, BorshSerialize};
use nomt_core::proof::MultiProof;
use serde::Serialize;

#[cfg(feature = "native")]
pub mod prover_storage;
pub mod zk_storage;

/// Wrapper around [`nomt_core::proof::MultiProof`] that implements `PartialEq`/`Eq`
/// by comparing borsh-serialized representations.
///
/// This is necessary because `MultiProof` doesn't derive these traits.
#[derive(Debug, Clone, Serialize, serde::Deserialize, BorshSerialize, BorshDeserialize)]
pub struct NomtMultiProof(pub MultiProof);

impl PartialEq for NomtMultiProof {
    fn eq(&self, other: &Self) -> bool {
        match (borsh::to_vec(&self.0), borsh::to_vec(&other.0)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for NomtMultiProof {}
