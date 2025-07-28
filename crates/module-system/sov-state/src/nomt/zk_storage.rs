//! ZK Verifier part of the NOMT based Storage implementation
use std::marker::PhantomData;

use nomt_core::hasher::BinaryHasher;
use nomt_core::proof::MultiProof;
use nomt_core::trie::{KeyPath, LeafData, Node, ValueHash};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::reexports::digest::Digest;

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

        let multi_proof: MultiProof = array_witness.get_hint();
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

        let mut updates = state_writes
            .into_iter()
            .map(|(key, value)| {
                (
                    S::Hasher::digest(key.as_ref()).into(),
                    value.map(|slot_value| {
                        // Authenticated write is hash of a combination of size and orignal value hash.
                        S::Hasher::digest(slot_value.combine_val_hash_and_size::<S::Hasher>())
                            .into()
                    }),
                )
            })
            .collect::<Vec<(KeyPath, Option<ValueHash>)>>();

        // Sort them by key hash, as required by [`nomt_core::proof::verify_multi_proof_update`]
        updates.sort_by(|a, b| a.0.cmp(&b.0));

        nomt_core::proof::verify_multi_proof_update::<BinaryHasher<S::Hasher>>(
            &verified_multi_proof,
            updates,
        )
        .map_err(|e| anyhow::anyhow!("Failed to verify update: {:?}", e))
        // Note: we don't check exhaustion of the proof
        // because it does not impact the correctness of the guest, only performance.
    }
}

impl<S: MerkleProofSpec> Storage for NomtVerifierStorage<S> {
    type Hasher = S::Hasher;
    type Witness = S::Witness;
    type Proof = ();
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

    fn get_accessory(&self, _key: &SlotKey, _version: Option<SlotNumber>) -> Option<SlotValue> {
        unimplemented!("The NomtZkStorage does not have the accessory state yet.")
    }

    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
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
        _state_root: Self::Root,
        _proof: StorageProof<Self::Proof>,
    ) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
        unimplemented!("The NomtZkStorage does not support `open_proof` yet.");
    }
}

#[cfg(feature = "test-utils")]
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

    fn get_with_proof<N: crate::namespaces::ProvableCompileTimeNamespace>(
        &self,
        _key: SlotKey,
        _version: Option<SlotNumber>,
    ) -> anyhow::Result<StorageProof<Self::Proof>> {
        unimplemented!("The NomtVerifierStorage should not be used to generate merkle proofs! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_historical<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        _version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> Option<SlotValue> {
        unimplemented!("The NomtVerifierStorage does not support `get_historical`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        _version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        unimplemented!("The NomtVerifierStorage does not support `get_leaf_historical`! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }

    fn get_root_hash(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        self.get_root_hash_unbound(version)
    }

    fn get_root_hash_unbound(&self, _version: SlotNumber) -> anyhow::Result<Self::Root> {
        unimplemented!("The NomtVerifierStorage should not be used to get root hash! The NativeStorage trait is only implemented to allow for the use of the NomtVerifierStorage in tests.");
    }
}
