//! Storage implementation based on [Nearly Optimal Merkle Tree(NOMT)](https://github.com/thrumdev/nomt/) implementation.

use borsh::{BorshDeserialize, BorshSerialize};
use nomt_core::hasher::BinaryHasher;
use nomt_core::proof::MultiProof;
use nomt_core::trie::{KeyPath, LeafData, Node, ValueHash};
use serde::Serialize;
use sov_rollup_interface::reexports::digest::Digest;

use crate::{MerkleProofSpec, SlotKey, SlotValue, StateRoot, StorageProof, StorageRoot};

#[cfg(feature = "native")]
pub mod prover_storage;
pub mod zk_storage;

/// Verifies a storage proof against a state root and returns the key-value pair.
///
/// This is shared logic used by both Nomt-based storages.
pub(crate) fn verify_storage_proof<S: MerkleProofSpec>(
    state_root: StorageRoot<S>,
    proof: StorageProof<NomtMultiProof>,
) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
    let StorageProof {
        key,
        value,
        proof: multi_proof,
        namespace,
    } = proof;
    let root_node: Node = state_root.namespace_root(namespace);

    let verified =
        nomt_core::proof::verify_multi_proof::<BinaryHasher<S::Hasher>>(&multi_proof.0, root_node)
            .map_err(|e| anyhow::anyhow!("Failed to verify proof: {:?}", e))?;

    let key_path: KeyPath = S::Hasher::digest(key.as_ref()).into();

    match &value {
        None => {
            if !verified
                .confirm_nonexistence(&key_path)
                .map_err(|e| anyhow::anyhow!("Key out of scope: {:?}", e))?
            {
                anyhow::bail!("Failed to verify non-existence of key");
            }
        }
        Some(slot_value) => {
            let authenticated_write = slot_value.combine_val_hash_and_size::<S::Hasher>();
            let value_hash: ValueHash = S::Hasher::digest(&authenticated_write).into();
            let leaf = LeafData {
                key_path,
                value_hash,
            };
            if !verified
                .confirm_value(&leaf)
                .map_err(|e| anyhow::anyhow!("Key out of scope: {:?}", e))?
            {
                anyhow::bail!("Failed to verify value for key");
            }
        }
    }

    Ok((key, value))
}

/// Wrapper around [`nomt_core::proof::MultiProof`] that implements `PartialEq`/`Eq`
/// by manually comparing each of inner attributes of [`nomt_core::proof::MultiProof`]
///
/// This is necessary because `MultiProof` doesn't derive these traits.
#[derive(Debug, Clone, Serialize, serde::Deserialize, BorshSerialize, BorshDeserialize)]
pub struct NomtMultiProof(pub MultiProof);

impl PartialEq for NomtMultiProof {
    fn eq(&self, other: &Self) -> bool {
        let this = &self.0;
        let other = &other.0;

        if this.paths.len() != other.paths.len() {
            return false;
        }

        if &this.siblings != &other.siblings {
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
