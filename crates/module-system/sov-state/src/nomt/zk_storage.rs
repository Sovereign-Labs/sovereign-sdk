//! ZK Verifier part of the NOMT based Storage implementation
use std::collections::BTreeMap;
use std::marker::PhantomData;

use nomt_core::hasher::BinaryHasher;
use nomt_core::proof::{MultiProof, PathUpdate};
use nomt_core::trie::{KeyPath, LeafData, Node, ValueHash};
use nomt_core::witness::Witness as NomtWitness;
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
        let mut expected_reads = BTreeMap::new();
        for (key, value) in state_reads {
            let key_hash: KeyPath = S::Hasher::digest(key.as_ref()).into();
            let value_hash = value.map(|node_leaf| {
                S::Hasher::digest(node_leaf.combine_val_hash_and_size()).into()
            });
            if expected_reads.insert(key_hash, value_hash).is_some() {
                anyhow::bail!("Duplicate key read in state accesses: {:?}", key);
            }
        }

        let mut expected_writes = BTreeMap::new();
        for (key, value) in state_writes {
            let key_hash: KeyPath = S::Hasher::digest(key.as_ref()).into();
            let value_hash = value.map(|slot_value| {
                // Authenticated write is hash of a combination of size and original value hash.
                S::Hasher::digest(slot_value.combine_val_hash_and_size::<S::Hasher>()).into()
            });
            if expected_writes.insert(key_hash, value_hash).is_some() {
                anyhow::bail!("Duplicate key write in state accesses: {:?}", key);
            }
        }

        let nomt_witness: NomtWitness = array_witness.get_hint();
        let mut path_proofs_inner = nomt_witness
            .path_proofs
            .iter()
            .map(|path| path.inner.clone())
            .collect::<Vec<_>>();
        path_proofs_inner.sort_by(|a, b| a.terminal.path().cmp(b.terminal.path()));
        let multi_proof = MultiProof::from_path_proofs(path_proofs_inner);
        let verified_multi_proof = nomt_core::proof::verify_multi_proof::<BinaryHasher<S::Hasher>>(
            &multi_proof,
            prev_root,
        )
        .map_err(|e| anyhow::anyhow!("Failed to verify multi proof: {:?}", e))?;

        for (key, value) in &expected_reads {
            match value {
                None => {
                    if !verified_multi_proof
                        .confirm_nonexistence(key)
                        .map_err(|e| anyhow::anyhow!("Failed to confirm non-existence: {:?}", e))?
                    {
                        anyhow::bail!("Failed to verify non-existence of key");
                    }
                }
                Some(value_hash) => {
                    let leaf = LeafData {
                        key_path: *key,
                        value_hash: *value_hash,
                    };
                    if !verified_multi_proof
                        .confirm_value(&leaf)
                        .map_err(|e| anyhow::anyhow!("Failed to confirm value: {:?}", e))?
                    {
                        anyhow::bail!("Failed to verify inclusion of key");
                    }
                }
            }
        }

        let mut witnessed_writes = BTreeMap::new();
        for write in nomt_witness.operations.writes {
            let Some(expected_value) = expected_writes.remove(&write.key) else {
                anyhow::bail!("Unexpected or duplicate NOMT write witness entry");
            };
            if write.value != expected_value {
                anyhow::bail!(
                    "Mismatched NOMT write witness value for key {:?}: witness={:?}, expected={:?}",
                    write.key,
                    write.value,
                    expected_value
                );
            }
            if witnessed_writes
                .insert(write.key, expected_value)
                .is_some()
            {
                anyhow::bail!("Duplicate NOMT write witness entry");
            };
        }

        if !expected_writes.is_empty() {
            anyhow::bail!("Missing NOMT write witness entries");
        }

        let mut verified_paths = nomt_witness
            .path_proofs
            .into_iter()
            .map(|path| {
                let verified = path
                    .inner
                    .verify::<BinaryHasher<S::Hasher>>(path.path.path(), prev_root)
                    .map_err(|e| anyhow::anyhow!("Failed to verify path proof: {:?}", e))?;
                Ok((
                    verified,
                    path.path.depth() as usize,
                    truncate_key_path(path.path.raw_path(), path.path.depth() as usize),
                    Vec::<(KeyPath, Option<ValueHash>)>::new(),
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        verified_paths.sort_by(|a, b| a.0.path().cmp(b.0.path()));

        let mut path_index_by_prefix = BTreeMap::new();
        let mut path_depths_desc = Vec::new();
        for (index, (_, depth, prefix, _)) in verified_paths.iter().enumerate() {
            if path_index_by_prefix
                .insert((*depth, *prefix), index)
                .is_some()
            {
                anyhow::bail!("Duplicate NOMT path proof prefix");
            }
            path_depths_desc.push(*depth);
        }
        path_depths_desc.sort_unstable();
        path_depths_desc.dedup();
        path_depths_desc.reverse();

        for (key, value) in witnessed_writes {
            let matching_index = path_depths_desc
                .iter()
                .find_map(|depth| {
                    path_index_by_prefix
                        .get(&(*depth, truncate_key_path(key, *depth)))
                        .copied()
                })
                .ok_or_else(|| anyhow::anyhow!("No NOMT path proof covers write key {:?}", key))?;

            verified_paths[matching_index].3.push((key, value));
        }

        let mut updates = Vec::new();
        for (verified, _, _, writes) in verified_paths {
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

fn truncate_key_path(mut key: KeyPath, depth: usize) -> KeyPath {
    debug_assert!(depth <= 256);

    let full_bytes = depth / 8;
    let partial_bits = depth % 8;

    if full_bytes < key.len() {
        if partial_bits == 0 {
            key[full_bytes..].fill(0);
        } else {
            let keep_mask = u8::MAX << (8 - partial_bits);
            key[full_bytes] &= keep_mask;
            key[full_bytes + 1..].fill(0);
        }
    }

    key
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
