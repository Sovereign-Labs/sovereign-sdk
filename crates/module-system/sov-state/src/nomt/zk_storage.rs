//! ZK Verifier part of the NOMT based Storage implementation
use std::marker::PhantomData;

use nomt_core::trie::Node;
#[cfg(all(feature = "test-utils", feature = "native"))]
use sov_rollup_interface::common::SlotNumber;

use super::{replay_state_update_witness, NomtStateProof};
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
        let state_proof: NomtStateProof = array_witness.get_hint();
        replay_state_update_witness::<S>(&state_accesses, &state_proof, prev_root)
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
