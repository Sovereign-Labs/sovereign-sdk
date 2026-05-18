//! Storage implementation based on [Nearly Optimal Merkle Tree(NOMT)](https://github.com/thrumdev/nomt/) implementation.

use nomt_core::hasher::BinaryHasher;
use nomt_core::proof::MultiProof;
use nomt_core::trie::{KeyPath, LeafData, Node, ValueHash};
use sov_rollup_interface::reexports::digest::Digest;

use crate::{MerkleProofSpec, SlotKey, SlotValue, StateRoot, StorageProof, StorageRoot};

#[cfg(feature = "native")]
pub mod prover_storage;
pub mod zk_storage;

/// Verifies a storage proof against a state root and returns the key-value pair.
///
/// This is shared logic used by both Nomt-based storages.
pub fn verify_storage_proof<S: MerkleProofSpec>(
    state_root: StorageRoot<S>,
    proof: StorageProof<MultiProof>,
) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
    let StorageProof {
        key,
        value,
        proof: multi_proof,
        namespace,
    } = proof;
    let root_node: Node = state_root.namespace_root(namespace);

    let verified =
        nomt_core::proof::verify_multi_proof::<BinaryHasher<S::Hasher>>(&multi_proof, root_node)
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
