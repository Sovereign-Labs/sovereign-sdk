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
        let this = &self.0;
        let other = &other.0;

        if &this.siblings != &other.siblings {
            return false;
        }

        if this.paths.len() != this.paths.len() {
            return false;
        }

        for (this_path, other_path) in this.paths.iter().zip(other.paths.iter()) {
            if this_path.depth != other_path.depth {
                return false;
            }

            if this_path.terminal != other_path.terminal {
                return false;
            }
        }

        true
    }
}

impl Eq for NomtMultiProof {}
