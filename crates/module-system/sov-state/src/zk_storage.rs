use std::marker::PhantomData;

use jmt::storage::TreeReader;
use jmt::JellyfishMerkleTree;
#[cfg(feature = "bench")]
use sov_modules_macros::cycle_tracker;
use sov_rollup_interface::common::SlotNumber;

use crate::cache::{OrderedReadsAndWrites, StateAccesses};
use crate::jmt::KeyHash;
use crate::namespaces::CompileTimeNamespace;
use crate::storage::{SlotKey, SlotValue, Storage, StorageProof};
use crate::storage_internals::SparseMerkleProof;
#[cfg(feature = "test-utils")]
use crate::ProvableCompileTimeNamespace;
use crate::{
    open_merkle_proof, MerkleProofSpec, NodeLeafAndMaybeValue, ProvableNamespace, ReadType,
    StateRoot, StorageRoot, Witness,
};

/// A [`Storage`] implementation designed to be used inside the zkVM.
#[derive(Default, derivative::Derivative)]
#[derivative(Clone(bound = "S: MerkleProofSpec"), Debug(bound = ""))]
pub struct ZkStorage<S: MerkleProofSpec> {
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec> ZkStorage<S> {
    /// Creates a new [`ZkStorage`] instance. Identical to [`Default::default`].
    pub fn new() -> Self {
        Self {
            _phantom_hasher: PhantomData,
        }
    }
}

#[cfg_attr(feature = "bench", cycle_tracker)]
fn jmt_verify_existence<S: MerkleProofSpec>(
    prev_state_root: [u8; 32],
    state_accesses: &OrderedReadsAndWrites,
    witness: &S::Witness,
) -> anyhow::Result<()> {
    // For each value that's been read from the tree, verify the provided smt proof
    for (key, read_value) in &state_accesses.ordered_reads {
        let key_hash = KeyHash::with::<S::Hasher>(key.key().as_ref());
        // This TODO is for performance enhancement, not a security concern.
        // TODO: Switch to the batch read API once it becomes available
        let proof: jmt::proof::SparseMerkleProof<S::Hasher> = witness.get_hint();

        match read_value {
            Some(node_leaf) => proof.verify_existence(
                jmt::RootHash(prev_state_root),
                key_hash,
                node_leaf.combine_val_hash_and_size(),
            )?,
            None => proof.verify_nonexistence(jmt::RootHash(prev_state_root), key_hash)?,
        }
    }

    Ok(())
}

#[cfg_attr(feature = "bench", cycle_tracker)]
fn jmt_verify_update<S: MerkleProofSpec>(
    prev_state_root: [u8; 32],
    state_accesses: OrderedReadsAndWrites,
    witness: &S::Witness,
) -> [u8; 32] {
    // Compute the jmt update from the write batch
    let batch = state_accesses
        .ordered_writes
        .into_iter()
        .map(|(key, value)| {
            let key_hash = KeyHash::with::<S::Hasher>(key.key().as_ref());
            let val_hash_and_size = value
                .as_ref()
                .map(SlotValue::combine_val_hash_and_size::<S::Hasher>);
            (key_hash, val_hash_and_size)
        })
        .collect::<Vec<_>>();

    let update_proof: jmt::proof::UpdateMerkleProof<S::Hasher> = witness.get_hint();
    let new_root: [u8; 32] = witness.get_hint();
    update_proof
        .verify_update(
            jmt::RootHash(prev_state_root),
            jmt::RootHash(new_root),
            batch,
        )
        .expect("Updates must be valid");

    new_root
}

impl<S: MerkleProofSpec> ZkStorage<S> {
    fn compute_state_update_namespace(
        state_accesses: OrderedReadsAndWrites,
        witness: &S::Witness,
        prev_state_root: jmt::RootHash,
    ) -> anyhow::Result<jmt::RootHash> {
        // For each value that's been read from the tree, verify the provided smt proof
        jmt_verify_existence::<S>(prev_state_root.0, &state_accesses, witness)?;

        let new_root = jmt_verify_update::<S>(prev_state_root.0, state_accesses, witness);

        Ok(jmt::RootHash(new_root))
    }
}

/// A helper struct for computing the empty root hash. We need this because the jmt puts a `TreeReader`
/// constraint on its DB to compute provide the empty root const, even though that reader is not actually touched.
struct EmptyTreeReader;
impl TreeReader for EmptyTreeReader {
    fn get_node_option(
        &self,
        _node_key: &jmt::storage::NodeKey,
    ) -> anyhow::Result<Option<jmt::storage::Node>> {
        Ok(None)
    }

    fn get_value_option(
        &self,
        _max_version: jmt::Version,
        _key_hash: KeyHash,
    ) -> anyhow::Result<Option<jmt::OwnedValue>> {
        Ok(None)
    }

    fn get_rightmost_leaf(
        &self,
    ) -> anyhow::Result<Option<(jmt::storage::NodeKey, jmt::storage::LeafNode)>> {
        Ok(None)
    }
}

impl<S: MerkleProofSpec> Storage for ZkStorage<S> {
    type Hasher = S::Hasher;
    type Witness = S::Witness;
    type Proof = SparseMerkleProof<S::Hasher>;
    type Root = StorageRoot<S>;
    type StateUpdate = ();
    type ChangeSet = ();

    const PRE_GENESIS_ROOT: Self::Root = StorageRoot::<S>::new(
        JellyfishMerkleTree::<EmptyTreeReader, S::Hasher>::EMPTY_ROOT.0,
        JellyfishMerkleTree::<EmptyTreeReader, S::Hasher>::EMPTY_ROOT.0,
    );

    fn get_accessory(&self, _key: &SlotKey, _version: Option<SlotNumber>) -> Option<SlotValue> {
        unimplemented!("The ZkStorage does not have access to the accessory state.")
    }

    fn put_in_witness(&self, _value: Option<SlotValue>, _witness: &Self::Witness) {}

    fn get_leaf<N: CompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        let leaf = witness.get_hint::<Option<NodeLeafAndMaybeValue>>()?;
        // The zk-storage does not pre-load the full value.
        assert_eq!(leaf.value, ReadType::GetSizeValueNotFetched);
        Some(leaf)
    }

    #[cfg_attr(feature = "bench", cycle_tracker)]
    fn get<N: CompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<SlotValue> {
        witness.get_hint()
    }

    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
    ) -> anyhow::Result<(Self::Root, Self::StateUpdate)> {
        let prev_user_root = prev_state_root.namespace_root(ProvableNamespace::User);
        let prev_kernel_root = prev_state_root.namespace_root(ProvableNamespace::Kernel);
        let user_root = ZkStorage::<S>::compute_state_update_namespace(
            state_accesses.user,
            witness,
            jmt::RootHash(prev_user_root),
        )?;
        let kernel_root = ZkStorage::<S>::compute_state_update_namespace(
            state_accesses.kernel,
            witness,
            jmt::RootHash(prev_kernel_root),
        )?;

        Ok((StorageRoot::<S>::new(user_root.0, kernel_root.0), ()))
    }

    fn materialize_changes(self, _node_batch: Self::StateUpdate) {}

    fn open_proof(
        state_root: Self::Root,
        state_proof: StorageProof<Self::Proof>,
    ) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
        open_merkle_proof(state_root, state_proof)
    }
}

#[cfg(feature = "test-utils")]
// `NativeStorage`` is implemented for `ZkStorage` solely for testing purposes.
// In some tests, we use both `ProverStorage`` and `ZkStorage`.
// Due to feature unification, we must provide this implementation even though it is not used.
impl<S: MerkleProofSpec> crate::storage::NativeStorage for ZkStorage<S> {
    fn latest_version(&self) -> SlotNumber {
        unimplemented!("Latest version is not available for ZkStorage");
    }

    fn latest_version_unbound(&self) -> SlotNumber {
        unimplemented!("Latest unbound version is not available for ZkStorage");
    }

    fn get_with_proof<N: crate::namespaces::ProvableCompileTimeNamespace>(
        &self,
        _key: SlotKey,
        _version: Option<SlotNumber>,
    ) -> anyhow::Result<StorageProof<Self::Proof>> {
        unimplemented!("The ZkStorage should not be used to generate merkle proofs! The NativeStorage trait is only implemented to allow for the use of the ZkStorage in tests.");
    }

    fn get_root_hash(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        self.get_root_hash_unbound(version)
    }

    fn get_historical<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        _version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> Option<SlotValue> {
        unimplemented!("The ZkStorage does not support `get_historical`! The NativeStorage trait is only implemented to allow for the use of the ZkStorage in tests.");
    }

    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        _key: &SlotKey,
        _version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        unimplemented!("The ZkStorage does not support `get_leaf_historical`! The NativeStorage trait is only implemented to allow for the use of the ZkStorage in tests.");
    }

    fn get_root_hash_unbound(&self, _version: SlotNumber) -> anyhow::Result<Self::Root> {
        unimplemented!("The ZkStorage should not be used to get root hash! The NativeStorage trait is only implemented to allow for the use of the ZkStorage in tests.");
    }
}
