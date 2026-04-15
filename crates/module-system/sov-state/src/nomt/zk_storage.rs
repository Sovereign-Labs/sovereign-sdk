//! ZK Verifier part of the NOMT based Storage implementation
use std::marker::PhantomData;

use nomt_core::hasher::BinaryHasher;
use nomt_core::proof::{MultiProof, PathProof, PathUpdate, VerifiedPathProof};
use nomt_core::trie::{KeyPath, LeafData, Node, ValueHash};
#[cfg(all(feature = "test-utils", feature = "native"))]
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::reexports::digest::Digest;

use crate::nomt::NomtMultiProof;
use crate::pinned_cache::PinnedCache;
use crate::storage::ReadType;
use crate::{
    MerkleProofSpec, NodeLeafAndMaybeValue, OrderedReadsAndWrites, ProvableCompileTimeNamespace,
    ProvableNamespace, SlotKey, SlotValue, StateAccesses, StateRoot, Storage, StorageProof,
    StorageRoot, Witness,
};

#[derive(Clone, Copy)]
struct PathProofRange {
    lower: usize,
    upper: usize,
    path_bit_index: usize,
}

fn validate_path_proofs_for_multi_proof(path_proofs: &[PathProof]) -> anyhow::Result<()> {
    for window in path_proofs.windows(2) {
        if window[0].terminal.path() >= window[1].terminal.path() {
            anyhow::bail!(
                "Malformed NOMT path proof hint: terminal paths must be strictly increasing"
            );
        }
    }

    if path_proofs.is_empty() {
        return Ok(());
    }

    let mut stack = vec![PathProofRange {
        lower: 0,
        upper: path_proofs.len(),
        path_bit_index: 0,
    }];

    while let Some(mut range) = stack.pop() {
        while range.lower + 1 < range.upper {
            let lower_path = path_proofs[range.lower].terminal.path();
            let upper_path = path_proofs[range.upper - 1].terminal.path();

            if lower_path.len() <= range.path_bit_index || upper_path.len() <= range.path_bit_index
            {
                anyhow::bail!(
                    "Malformed NOMT path proof hint: terminal path ended before range divergence"
                );
            }

            if lower_path[range.path_bit_index] != upper_path[range.path_bit_index] {
                let mut mid = None;
                for idx in range.lower..range.upper {
                    let path = path_proofs[idx].terminal.path();
                    if path.len() <= range.path_bit_index {
                        anyhow::bail!(
                            "Malformed NOMT path proof hint: terminal path ended before range divergence"
                        );
                    }
                    if mid.is_none() && path[range.path_bit_index] {
                        mid = Some(idx);
                    }
                }

                let Some(mid) = mid else {
                    anyhow::bail!(
                        "Malformed NOMT path proof hint: could not bisect path proof range"
                    );
                };

                stack.push(PathProofRange {
                    lower: mid,
                    upper: range.upper,
                    path_bit_index: range.path_bit_index + 1,
                });
                range.upper = mid;
                range.path_bit_index += 1;
                continue;
            }

            if path_proofs[range.lower].siblings.len() <= range.path_bit_index {
                anyhow::bail!(
                    "Malformed NOMT path proof hint: missing sibling at shared depth {}",
                    range.path_bit_index
                );
            }

            range.path_bit_index += 1;
        }
    }

    Ok(())
}

/// A [`Storage`] implementation designed to be used inside the zkVM, based on NOMT.
#[derive(Default, derivative::Derivative)]
#[derivative(Clone(bound = "S: MerkleProofSpec"), Debug(bound = ""))]
pub struct NomtVerifierStorage<S: MerkleProofSpec> {
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec> NomtVerifierStorage<S> {
    /// Creates a new [`NomtVerifierStorage`] instance. Identical to [`Default::default`].
    pub fn new() -> Self {
        Self {
            _phantom_hasher: Default::default(),
        }
    }

    fn compute_state_update_namespace(
        state_accesses: OrderedReadsAndWrites,
        array_witness: &S::Witness,
        prev_root: Node,
    ) -> anyhow::Result<Node> {
        let OrderedReadsAndWrites {
            ordered_reads: state_reads,
            ordered_writes: state_writes,
        } = state_accesses;

        // The hint is UNTRUSTED. Every `PathProof` is authenticated against the trusted
        // `prev_root` below: per-path via `PathProof::verify` (used for writes) and in
        // aggregate via `verify_multi_proof` (used for reads). Write values are hashed
        // from the trusted `state_writes` (execution trace) — witness-reported values are
        // never used.
        let mut path_proofs: Vec<PathProof> = array_witness.get_hint();
        // Sort once: `MultiProof::from_path_proofs` expects terminal-path order, and
        // `verify_update` below also expects its paths ascending — same key, one sort.
        path_proofs.sort_unstable_by(|a, b| a.terminal.path().cmp(b.terminal.path()));
        validate_path_proofs_for_multi_proof(&path_proofs)?;

        // Reads: rebuild a MultiProof from the path proofs and verify against prev_root,
        // then confirm each read via the existing NOMT primitives.
        let multi_proof = MultiProof::from_path_proofs(path_proofs.clone());
        let verified_multi_proof = nomt_core::proof::verify_multi_proof::<BinaryHasher<S::Hasher>>(
            &multi_proof,
            prev_root,
        )
        .map_err(|e| anyhow::anyhow!("Failed to verify multi proof: {:?}", e))?;

        for (key, value) in state_reads {
            let key_hash: KeyPath = S::Hasher::digest(key.as_ref()).into();
            match value {
                None => {
                    if !verified_multi_proof
                        .confirm_nonexistence(&key_hash)
                        .map_err(|e| anyhow::anyhow!("Failed to confirm non-existence: {:?}", e))?
                    {
                        anyhow::bail!("Failed to verify non-existence of key: {:?}", key);
                    }
                }
                Some(node_leaf) => {
                    let authenticated_write = node_leaf.combine_val_hash_and_size();
                    let value_hash = S::Hasher::digest(&authenticated_write).into();
                    let leaf = LeafData {
                        key_path: key_hash,
                        value_hash,
                    };
                    if !verified_multi_proof
                        .confirm_value(&leaf)
                        .map_err(|e| anyhow::anyhow!("Failed to confirm value: {:?}", e))?
                    {
                        anyhow::bail!("Failed to verify inclusion of key: {:?}", key);
                    }
                }
            }
        }

        // Writes: verify each path against prev_root, then route each trusted write into
        // its single covering path and call verify_update (the per-path update algorithm
        // that mirrors the prover's `Session::finish().root()`). `path_proofs` is already
        // in ascending path order, so `verified_paths` inherits the ordering that
        // `verify_update`'s PathsOutOfOrder check requires.
        let mut verified_paths: Vec<(VerifiedPathProof, Vec<(KeyPath, Option<ValueHash>)>)> =
            path_proofs
                .into_iter()
                .map(|pp| {
                    let verified = pp
                        .verify::<BinaryHasher<S::Hasher>>(pp.terminal.path(), prev_root)
                        .map_err(|e| anyhow::anyhow!("Failed to verify path proof: {:?}", e))?;
                    Ok((verified, Vec::new()))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;

        // Hash and sort writes globally. Disjoint Patricia paths ⇒ each write routes to
        // exactly one bucket, and global ascending key order ⇒ each bucket ends up
        // locally ascending (OpsOutOfOrder check).
        let mut sorted_writes: Vec<(KeyPath, Option<ValueHash>)> = state_writes
            .into_iter()
            .map(|(key, value)| {
                let key_hash: KeyPath = S::Hasher::digest(key.as_ref()).into();
                let value_hash = value.map(|slot_value| {
                    // Authenticated write is hash of a combination of size and original value hash.
                    S::Hasher::digest(slot_value.combine_val_hash_and_size::<S::Hasher>()).into()
                });
                (key_hash, value_hash)
            })
            .collect();
        sorted_writes.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        // Route writes with a single forward sweep. Both vectors are globally sorted, and
        // each write belongs to exactly one disjoint verified path, so once a key falls
        // past a path we never need to revisit it.
        let mut path_idx = 0;
        for (key_hash, value_hash) in sorted_writes {
            while path_idx < verified_paths.len()
                && verified_paths[path_idx]
                    .0
                    .confirm_nonexistence(&key_hash)
                    .is_err()
            {
                path_idx += 1;
            }

            let Some((_, writes)) = verified_paths.get_mut(path_idx) else {
                anyhow::bail!("No NOMT path proof covers write key: {:?}", key_hash);
            };
            writes.push((key_hash, value_hash));
        }

        let mut updates = Vec::new();
        for (verified, writes) in verified_paths {
            if !writes.is_empty() {
                updates.push(PathUpdate {
                    inner: verified,
                    ops: writes,
                });
            }
        }

        nomt_core::proof::verify_update::<BinaryHasher<S::Hasher>>(prev_root, &updates)
            .map_err(|e| anyhow::anyhow!("Failed to verify update: {:?}", e))
    }
}

impl<S: MerkleProofSpec> Storage for NomtVerifierStorage<S> {
    type Hasher = S::Hasher;
    type Witness = S::Witness;
    type Proof = NomtMultiProof;
    type Root = StorageRoot<S>;
    type StateUpdate = ();
    type ChangeSet = ();
    const PRE_GENESIS_ROOT: Self::Root =
        StorageRoot::new(nomt_core::trie::TERMINATOR, nomt_core::trie::TERMINATOR);

    fn put_in_witness(&self, _value: Option<SlotValue>, _witness: &Self::Witness) {}

    fn get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        let leaf = witness.get_hint::<Option<NodeLeafAndMaybeValue>>()?;
        // The zk-storage does not preload the full value.
        assert_eq!(leaf.value, ReadType::GetSizeValueNotFetched);
        Some(leaf)
    }

    fn get<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<SlotValue> {
        witness.get_hint()
    }

    fn get_accessory(&self, _key: &SlotKey) -> Option<SlotValue> {
        unimplemented!("The NomtZkStorage does not have the accessory state yet.")
    }

    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
        _pinned_cache: Option<PinnedCache>,
    ) -> anyhow::Result<(Self::Root, Self::StateUpdate)> {
        let StateAccesses { user, kernel } = state_accesses;

        let prev_user_root = prev_state_root.namespace_root(ProvableNamespace::User);
        let user_root_raw = Self::compute_state_update_namespace(user, witness, prev_user_root)?;
        let prev_kernel_root = prev_state_root.namespace_root(ProvableNamespace::Kernel);
        let kernel_root_raw =
            Self::compute_state_update_namespace(kernel, witness, prev_kernel_root)?;

        let root = StorageRoot::new(user_root_raw, kernel_root_raw);
        Ok((root, ()))
    }

    fn materialize_changes(self, _state_update: Self::StateUpdate) -> Self::ChangeSet {}

    fn open_proof(
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
        crate::nomt::verify_storage_proof::<S>(state_root, proof)
    }
}

#[cfg(all(feature = "test-utils", feature = "native"))]
// `NativeStorage`` is implemented for `ZkStorage` solely for testing purposes.
// In some tests, we use both `ProverStorage`` and `ZkStorage`.
// Due to feature unification, we must provide this implementation even though it is not used.
impl<S: MerkleProofSpec> crate::storage::NativeStorage for NomtVerifierStorage<S> {
    fn latest_version(&self) -> SlotNumber {
        unimplemented!("Latest version is not available for NomtVerifierStorage.");
    }

    fn latest_version_unbound(&self) -> SlotNumber {
        unimplemented!("Latest unbound version is not available for NomtVerifierStorage.");
    }

    fn get_historical<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        _version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> anyhow::Result<Option<SlotValue>> {
        unimplemented!("The NomtVerifierStorage does not support `get_historical`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        _version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> anyhow::Result<Option<NodeLeafAndMaybeValue>> {
        unimplemented!("The NomtVerifierStorage does not support `get_leaf_historical`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_accessory_historical(
        &self,
        _key: &SlotKey,
        _version: Option<SlotNumber>,
    ) -> anyhow::Result<Option<SlotValue>> {
        unimplemented!("The NomtVerifierStorage does not support `get_accessory_historical`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_with_proof<N: crate::namespaces::ProvableCompileTimeNamespace>(
        &self,
        _key: SlotKey,
    ) -> anyhow::Result<(StorageProof<Self::Proof>, SlotNumber, Self::Root)> {
        unimplemented!("The NomtVerifierStorage should not be used to generate merkle proofs! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_root_hash(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        self.get_root_hash_unbound(version)
    }

    fn get_root_hash_unbound(&self, _version: SlotNumber) -> anyhow::Result<Self::Root> {
        unimplemented!("The NomtVerifierStorage should not be used to get root hash! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_unbound<N: crate::CompileTimeNamespace>(&self, _key: SlotKey) -> Option<SlotValue> {
        unimplemented!("The NomtVerifierStorage does not support `get_unbound`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_accessory_unbound(
        &self,
        _key: SlotKey,
        _max_version: Option<SlotNumber>,
    ) -> Option<SlotValue> {
        unimplemented!("The NomtVerifierStorage does not support `get_accessory_unbound`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn maybe_iter_user_values_with_prefix(
        &self,
        _prefix: SlotKey,
    ) -> anyhow::Result<Option<impl Iterator<Item = (SlotKey, SlotValue)>>> {
        unimplemented!("The NomtVerifierStorage does not support `iter_with_prefix`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
        // We have to put this here to allow type inference, but we prefer to panic since calling this method is a bug.
        #[allow(unreachable_code)]
        Ok(Option::<std::iter::Once<(SlotKey, SlotValue)>>::None)
    }

    fn maybe_iter_kernel_values_with_prefix(
        &self,
        _prefix: SlotKey,
    ) -> anyhow::Result<Option<impl Iterator<Item = (SlotKey, SlotValue)>>> {
        unimplemented!("The NomtVerifierStorage does not support `iter_with_prefix`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
        // We have to put this here to allow type inference, but we prefer to panic since calling this method is a bug.
        #[allow(unreachable_code)]
        Ok(Option::<std::iter::Once<(SlotKey, SlotValue)>>::None)
    }

    fn try_load_saved_pinned_cache(&mut self) -> Option<PinnedCache> {
        unimplemented!("The NomtVerifierStorage does not support `take_pinned_cache`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }
}

#[cfg(test)]
mod tests {
    use nomt_core::proof::{PathProof, PathProofTerminal};
    use nomt_core::trie_pos::TriePosition;
    use sha2::Sha256;

    use super::{validate_path_proofs_for_multi_proof, NomtVerifierStorage};
    use crate::{ArrayWitness, DefaultStorageSpec, OrderedReadsAndWrites, Witness};

    #[test]
    fn validate_path_proofs_rejects_duplicate_terminal_paths() {
        let path_proof = PathProof {
            terminal: PathProofTerminal::Terminator(TriePosition::from_path_and_depth(
                [0; 32], 256,
            )),
            siblings: Vec::new(),
        };

        let err = validate_path_proofs_for_multi_proof(&[path_proof.clone(), path_proof])
            .expect_err("duplicate terminal paths should be rejected");

        assert!(err
            .to_string()
            .contains("terminal paths must be strictly increasing"));
    }

    #[test]
    fn validate_path_proofs_accepts_distinct_terminal_paths() {
        let left = PathProof {
            terminal: PathProofTerminal::Terminator(TriePosition::from_path_and_depth(
                [0; 32], 256,
            )),
            siblings: Vec::new(),
        };
        let mut right_key = [0; 32];
        right_key[0] = 0b1000_0000;
        let right = PathProof {
            terminal: PathProofTerminal::Terminator(TriePosition::from_path_and_depth(
                right_key, 256,
            )),
            siblings: Vec::new(),
        };

        validate_path_proofs_for_multi_proof(&[left, right])
            .expect("distinct terminal paths should remain valid");
    }

    #[test]
    fn compute_state_update_namespace_rejects_malformed_untrusted_hints() {
        type TestSpec = DefaultStorageSpec<Sha256>;

        let witness = ArrayWitness::default();
        let path_proof = PathProof {
            terminal: PathProofTerminal::Terminator(TriePosition::from_path_and_depth(
                [0; 32], 256,
            )),
            siblings: Vec::new(),
        };
        witness.add_hint(&vec![path_proof.clone(), path_proof]);

        let err = NomtVerifierStorage::<TestSpec>::compute_state_update_namespace(
            OrderedReadsAndWrites::default(),
            &witness,
            nomt_core::trie::TERMINATOR,
        )
        .expect_err("malformed path proof hints should return an error");

        assert!(err
            .to_string()
            .contains("terminal paths must be strictly increasing"));
    }
}
