//! Storage implementation based on [Nearly Optimal Merkle Tree(NOMT)](https://github.com/thrumdev/nomt/) implementation.

use std::collections::{BTreeMap, BTreeSet};

use borsh::{BorshDeserialize, BorshSerialize};
use nomt_core::hasher::BinaryHasher;
use nomt_core::proof::{MultiProof, PathUpdate};
use nomt_core::trie::{KeyPath, LeafData, Node, ValueHash};
use serde::Serialize;
use sov_rollup_interface::reexports::digest::Digest;

use crate::{
    MerkleProofSpec, OrderedReadsAndWrites, SlotKey, SlotValue, StateRoot, StorageProof,
    StorageRoot,
};

#[cfg(feature = "native")]
pub mod prover_storage;
pub mod zk_storage;

/// Verifies a storage proof against a state root and returns the key-value pair.
///
/// This is shared logic used by both Nomt-based storages.
pub fn verify_storage_proof<S: MerkleProofSpec>(
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

        if this.siblings != other.siblings {
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

type NomtStateProof = nomt_core::witness::Witness;

fn replay_state_update_witness<S: MerkleProofSpec>(
    state_accesses: &OrderedReadsAndWrites,
    state_proof: &NomtStateProof,
    prev_root: Node,
) -> anyhow::Result<Node> {
    let verified_paths = state_proof
        .path_proofs
        .iter()
        .map(|path_proof| {
            path_proof
                .inner
                .verify::<BinaryHasher<S::Hasher>>(path_proof.path.path(), prev_root)
                .map_err(|e| anyhow::anyhow!("Failed to verify path proof: {:?}", e))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut witnessed_reads = BTreeSet::<KeyPath>::new();
    for read in &state_proof.operations.reads {
        witnessed_reads.insert(read.key);
    }

    for (key, value) in &state_accesses.ordered_reads {
        let key_hash: KeyPath = S::Hasher::digest(key.as_ref()).into();

        match value {
            None => {
                if !witnessed_reads.contains(&key_hash) {
                    anyhow::bail!("Failed to verify non-existence of key: {:?}", key);
                }
                let mut proved_nonexistence = false;
                for verified_path in &verified_paths {
                    if verified_path.confirm_nonexistence(&key_hash).unwrap_or(false) {
                        proved_nonexistence = true;
                        break;
                    }
                }
                if !proved_nonexistence {
                    anyhow::bail!("Failed to verify non-existence of key: {:?}", key);
                }
            }
            Some(node_leaf) => {
                if !witnessed_reads.contains(&key_hash) {
                    anyhow::bail!("Failed to verify inclusion of key: {:?}", key);
                }
                let authenticated_write = node_leaf.combine_val_hash_and_size();
                let value_hash = S::Hasher::digest(&authenticated_write).into();
                let leaf = LeafData {
                    key_path: key_hash,
                    value_hash,
                };
                let mut proved_value = false;
                for verified_path in &verified_paths {
                    if verified_path.confirm_value(&leaf).unwrap_or(false) {
                        proved_value = true;
                        break;
                    }
                }
                if !proved_value {
                    anyhow::bail!("Failed to verify inclusion of key: {:?}", key);
                }
            }
        }
    }

    let mut witnessed_writes = BTreeMap::<(KeyPath, Option<ValueHash>), Vec<usize>>::new();
    for write in &state_proof.operations.writes {
        witnessed_writes
            .entry((write.key, write.value))
            .or_default()
            .push(write.path_index);
    }

    let mut writes_by_path_index = BTreeMap::<usize, Vec<(KeyPath, Option<ValueHash>)>>::new();
    for (key, value) in &state_accesses.ordered_writes {
        let key_hash: KeyPath = S::Hasher::digest(key.as_ref()).into();
        let expected_value = value.as_ref().map(|slot_value| {
            let authenticated_write = slot_value.combine_val_hash_and_size::<S::Hasher>();
            S::Hasher::digest(&authenticated_write).into()
        });
        let witness_path_indices = witnessed_writes
            .get_mut(&(key_hash, expected_value))
            .ok_or_else(|| anyhow::anyhow!("Missing witnessed write for key: {:?}", key))?;
        let hinted_path_index = witness_path_indices.pop();
        let path_index = hinted_path_index
            .filter(|path_index| {
                state_proof.path_proofs[*path_index]
                    .path
                    .subtrie_contains(&key_hash)
            })
            .or_else(|| {
                state_proof
                    .path_proofs
                    .iter()
                    .enumerate()
                    .filter(|(_, path_proof)| path_proof.path.subtrie_contains(&key_hash))
                    .max_by_key(|(_, path_proof)| path_proof.path.depth())
                    .map(|(path_index, _)| path_index)
            })
            .ok_or_else(|| anyhow::anyhow!("Missing witnessed write for key: {:?}", key))?;
        writes_by_path_index
            .entry(path_index)
            .or_default()
            .push((key_hash, expected_value));
    }

    let mut path_updates = writes_by_path_index
        .into_iter()
        .map(|(path_index, mut ops)| {
            ops.sort_by(|a, b| a.0.cmp(&b.0));
            let inner = verified_paths.get(path_index).cloned().ok_or_else(|| {
                anyhow::anyhow!("Witness write path index {} is out of bounds", path_index)
            })?;
            Ok(PathUpdate { inner, ops })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    path_updates.sort_by(|a, b| a.inner.path().cmp(b.inner.path()));

    nomt_core::proof::verify_update::<BinaryHasher<S::Hasher>>(prev_root, &path_updates)
        .map_err(|e| anyhow::anyhow!("Failed to verify update: {:?}", e))
}
