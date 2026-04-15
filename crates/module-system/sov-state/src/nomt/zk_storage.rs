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
        let path_proofs: Vec<PathProof> = array_witness.get_hint();

        // Reads: rebuild a MultiProof from the path proofs and verify against prev_root,
        // then confirm each read via the existing NOMT primitives.
        let mut sorted_path_proofs = path_proofs.clone();
        sorted_path_proofs.sort_by(|a, b| a.terminal.path().cmp(b.terminal.path()));
        let multi_proof = MultiProof::from_path_proofs(sorted_path_proofs);
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
        // that mirrors the prover's `Session::finish().root()`).
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
        // verify_update requires paths in ascending order (PathsOutOfOrder check).
        verified_paths.sort_by(|a, b| a.0.path().cmp(b.0.path()));

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
        sorted_writes.sort_by(|a, b| a.0.cmp(&b.0));

        for (key_hash, value_hash) in sorted_writes {
            // `confirm_nonexistence` returns `Err(KeyOutOfScope)` iff the key's bits do
            // NOT extend this path's proven bit-prefix — use it as an in-scope test.
            let idx = verified_paths
                .iter()
                .position(|(vp, _)| vp.confirm_nonexistence(&key_hash).is_ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("No NOMT path proof covers write key: {:?}", key_hash)
                })?;
            verified_paths[idx].1.push((key_hash, value_hash));
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
