use std::collections::BTreeMap;
use std::marker::PhantomData;

use anyhow::Context;
use jmt::{JellyfishMerkleTree, KeyHash};
use sov_db::accessory_db::AccessoryDb;
use sov_db::namespaces;
use sov_db::namespaces::{KernelNamespace as DBKernelNamespace, UserNamespace as DBUserNamespace};
use sov_db::state_db::{JmtHandler, StateDb, StateTreeChanges};
use sov_db::storage_manager::{InitializableNativeStorage, NativeChangeSet, StfStorageHandlers};
use sov_rollup_interface::common::SlotNumber;

use crate::cache::{OrderedReadsAndWrites, StateAccesses};
use crate::namespaces::{
    Accessory, CompileTimeNamespace, Namespace, ProvableCompileTimeNamespace, ProvableNamespace,
};
use crate::storage::{NativeStorage, SlotKey, SlotValue, StateUpdate, Storage, StorageProof};
use crate::storage_internals::{SparseMerkleProof, StorageRoot};
use crate::{
    open_merkle_proof, MerkleProofSpec, NodeLeaf, NodeLeafAndMaybeValue, ReadType, StateRoot,
    Witness,
};

/// A [`Storage`] implementation to be used by the prover in a native execution
/// environment (outside of the zkVM).
#[derive(derivative::Derivative)]
#[derivative(
    Clone(bound = "S: MerkleProofSpec"),
    Debug(bound = "S: MerkleProofSpec")
)]
pub struct ProverStorage<S: MerkleProofSpec> {
    db: StateDb,
    accessory_db: AccessoryDb,
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec> From<StfStorageHandlers> for ProverStorage<S> {
    fn from(value: StfStorageHandlers) -> Self {
        ProverStorage::with_db_handles(value.state, value.accessory)
    }
}

impl<S: MerkleProofSpec> InitializableNativeStorage for ProverStorage<S> {
    fn new(db: StateDb, accessory_db: AccessoryDb) -> Self {
        Self {
            db,
            accessory_db,
            _phantom_hasher: Default::default(),
        }
    }
}

impl<S: MerkleProofSpec> ProverStorage<S> {
    /// Creates a new [`ProverStorage`] instance from specified db handles
    pub fn with_db_handles(db: StateDb, accessory_db: AccessoryDb) -> Self {
        Self {
            db,
            accessory_db,
            _phantom_hasher: Default::default(),
        }
    }

    fn read_value_namespace<N: namespaces::Namespace>(
        &self,
        key: &SlotKey,
        version: SlotNumber,
    ) -> Option<SlotValue> {
        match self.db.get_value_option_by_key::<N>(version, key.as_ref()) {
            Ok(value) => value.map(Into::into),
            // It is ok to panic here, we assume the db is available and consistent.
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }

    fn do_get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &<Self as Storage>::Witness,
        version: Option<SlotNumber>,
    ) -> Option<NodeLeafAndMaybeValue> {
        let val = self.read_value::<N>(key, version);

        // First, we create a node that we put in the cache. This one contains the value.
        let node_leaf_with_fetched_value = val.map(|v| {
            let leaf = NodeLeaf::make_leaf::<S::Hasher>(&v);
            NodeLeafAndMaybeValue {
                leaf,
                value: ReadType::GetSizeValueFetched(v),
            }
        });

        // Second, we create a node that we put in the witness. This one doesn't contain the value.
        let node_leaf_without_value =
            node_leaf_with_fetched_value
                .clone()
                .map(|node| NodeLeafAndMaybeValue {
                    leaf: node.leaf,
                    value: ReadType::GetSizeValueNotFetched,
                });

        witness.add_hint(&node_leaf_without_value);
        node_leaf_with_fetched_value
    }

    // Return version to use, if applicable
    // If none returned, means no version can be used and should return empty value
    fn get_version_to_use(&self, version: Option<SlotNumber>) -> Option<SlotNumber> {
        if self.is_empty() {
            return None;
        }
        let next_version = self.db.get_next_version();
        match version {
            None => Some(
                next_version
                    .checked_sub(1)
                    .expect("Next version for non empty storage should be above 0"),
            ),
            Some(passed_version) => {
                if passed_version >= next_version {
                    None
                } else {
                    Some(passed_version)
                }
            }
        }
    }

    fn read_value<N: CompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
    ) -> Option<SlotValue> {
        let version_to_use = self.get_version_to_use(version)?;

        match N::NAMESPACE {
            Namespace::User => self.read_value_namespace::<DBUserNamespace>(key, version_to_use),
            Namespace::Kernel => {
                self.read_value_namespace::<DBKernelNamespace>(key, version_to_use)
            }
            Namespace::Accessory => self
                .accessory_db
                .get_value_option(key.as_ref(), version_to_use)
                .expect("Unable to read from AccessoryDb")
                .map(Into::into),
        }
    }

    fn get_root_hash_namespace_helper<N: namespaces::Namespace>(
        &self,
        version: SlotNumber,
    ) -> anyhow::Result<jmt::RootHash> {
        let state_db_handler: JmtHandler<N> = self.db.get_jmt_handler();
        let merkle = JellyfishMerkleTree::<JmtHandler<N>, S::Hasher>::new(&state_db_handler);
        merkle.get_root_hash(version.get())
    }

    pub(crate) fn compute_state_update_namespace<N: namespaces::Namespace>(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &<ProverStorage<S> as Storage>::Witness,
        prev_state_root: jmt::RootHash,
    ) -> anyhow::Result<(jmt::RootHash, ProverStateUpdate)> {
        let jmt_handler: JmtHandler<N> = self.db.get_jmt_handler();
        let jmt = JellyfishMerkleTree::<JmtHandler<N>, S::Hasher>::new(&jmt_handler);

        match self.db.get_next_version().checked_sub(1) {
            // If next_version is zero it means genesis
            None => (),
            // Previous root and reads are not witnessed during genesis.
            Some(latest_version) => {
                let stored_root_hash = jmt
                    .get_root_hash(latest_version.get())
                    .expect("Previous root hash was not populated");
                assert_eq!(
                    stored_root_hash, prev_state_root,
                    "Previous root hash does not match stored root hash. This is a bug."
                );
                // For each value that's been read from the tree, read it from the logged JMT to populate hints
                for (key, read_node_leaf) in &state_accesses.ordered_reads {
                    let key_hash = KeyHash::with::<S::Hasher>(key.key().as_ref());
                    // This TODO is for performance enhancement, not a security concern.
                    // TODO: Switch to the batch read API once it becomes available
                    let (value_from_proof, proof) =
                        jmt.get_with_proof(key_hash, latest_version.get())?;

                    let node_leaf_hash_and_size = read_node_leaf
                        .as_ref()
                        .map(|node| node.combine_val_hash_and_size());

                    let val_hash_and_size = value_from_proof.map(|value| {
                        SlotValue::from(value).combine_val_hash_and_size::<S::Hasher>()
                    });

                    if val_hash_and_size != node_leaf_hash_and_size {
                        anyhow::bail!(
                            "Bug! Incorrect value read from jmt. key={}, value_hash_and_size={:?}, node_leaf_hash_and_size={:?}",
                            key,
                            val_hash_and_size.map(|v| hex::encode(&v[..])),
                            node_leaf_hash_and_size.map(|leaf| hex::encode(
                                &leaf[..]
                            ))
                        );
                    }
                    witness.add_hint(&proof);
                }
            }
        };

        let mut key_preimages = Vec::with_capacity(state_accesses.ordered_writes.len());

        let mut original_writes = BTreeMap::default();
        let next_version = self.db.get_next_version().get();

        // Compute the JMT update from the batch of write operations.
        let batch = state_accesses
            .ordered_writes
            .into_iter()
            .map(|(key, value)| {
                let key_hash = KeyHash::with::<S::Hasher>(key.key().as_ref());
                key_preimages.push((key_hash, key.clone()));

                // Here we preserve the original wrtes that will be stored in the db.
                let original_write = value.as_ref().map(|v| v.value().to_vec());
                original_writes.insert((next_version, key_hash), original_write);

                let node_leaf_hash_and_size = value
                    .as_ref()
                    .map(|v| v.combine_val_hash_and_size::<S::Hasher>());

                (key_hash, node_leaf_hash_and_size)
            });

        let (new_root, update_proof, tree_update) = jmt
            .put_value_set_with_proof(batch, next_version)
            .expect("JMT update must succeed");

        witness.add_hint(&update_proof);
        witness.add_hint(&new_root.0);

        let new_state_update = ProverStateUpdate {
            data_to_materialize: StateTreeChanges {
                node_batch: tree_update.node_batch,
                original_write_values: original_writes,
            },
            key_preimages,
        };

        Ok((new_root, new_state_update))
    }

    fn materialize_accessory(
        &self,
        accessory_writes: &OrderedReadsAndWrites,
    ) -> sov_db::schema::SchemaBatch {
        let next_version = self.db.get_next_version();
        AccessoryDb::materialize_values(
            accessory_writes
                .ordered_writes
                .iter()
                .map(|(k, v_opt)| (k.key().to_vec(), v_opt.as_ref().map(|v| v.value().to_vec()))),
            next_version,
        )
        .expect("accessory db materialization must succeed")
    }

    // Caller must only use `Some(version)` from `get_version_to_use`.
    // Otherwise it might panic.
    fn get_with_proof_namespace<N: namespaces::Namespace>(
        &self,
        namespace: ProvableNamespace,
        key: SlotKey,
        version: SlotNumber,
    ) -> StorageProof<<ProverStorage<S> as Storage>::Proof> {
        let state_db_handler: JmtHandler<N> = self.db.get_jmt_handler();
        let merkle = JellyfishMerkleTree::<JmtHandler<N>, S::Hasher>::new(&state_db_handler);
        // We should've checked all input before this point, so any error means a bug.
        let (val_opt, proof) = merkle
            .get_with_proof(KeyHash::with::<S::Hasher>(key.as_ref()), version.get())
            .expect("Corrupted JMT state");
        StorageProof {
            key,
            value: val_opt.map(SlotValue::from),
            proof: SparseMerkleProof::<S::Hasher>::from(proof),
            namespace,
        }
    }

    /// Utility method for checking if storage is empty.
    /// Does not guarantee 100% that it actually is.
    pub fn is_empty(&self) -> bool {
        self.db.get_next_version() == SlotNumber::GENESIS
    }
}

#[derive(Default)]
pub struct ProverStateUpdate {
    pub(crate) data_to_materialize: StateTreeChanges,
    pub(crate) key_preimages: Vec<(KeyHash, SlotKey)>,
}

pub struct NamespacedStateUpdate {
    pub user: ProverStateUpdate,
    pub kernel: ProverStateUpdate,
    pub accessory: OrderedReadsAndWrites,
}

impl StateUpdate for NamespacedStateUpdate {
    fn add_accessory_item(&mut self, key: SlotKey, value: Option<SlotValue>) {
        self.accessory.ordered_writes.push((key, value));
    }

    fn get_accessory_items(&self) -> impl Iterator<Item = &(SlotKey, Option<SlotValue>)> {
        self.accessory.ordered_writes.iter()
    }
}

impl NamespacedStateUpdate {
    pub fn new(
        user: ProverStateUpdate,
        kernel: ProverStateUpdate,
        accessory: OrderedReadsAndWrites,
    ) -> Self {
        Self {
            user,
            kernel,
            accessory,
        }
    }
}

impl<S: MerkleProofSpec> Storage for ProverStorage<S> {
    type Hasher = S::Hasher;
    type Witness = S::Witness;
    type Proof = SparseMerkleProof<S::Hasher>;
    type Root = StorageRoot<S>;
    type StateUpdate = NamespacedStateUpdate;
    type ChangeSet = NativeChangeSet;

    const PRE_GENESIS_ROOT: Self::Root = StorageRoot::<S>::new(
        JellyfishMerkleTree::<JmtHandler<DBUserNamespace>, S::Hasher>::EMPTY_ROOT.0,
        JellyfishMerkleTree::<JmtHandler<DBUserNamespace>, S::Hasher>::EMPTY_ROOT.0,
    );

    fn put_in_witness(&self, value: Option<SlotValue>, witness: &Self::Witness) {
        witness.add_hint(&value);
    }

    fn get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        self.do_get_leaf::<N>(key, witness, None)
    }

    fn get<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<SlotValue> {
        let val = self.read_value::<N>(key, None);
        witness.add_hint(&val);
        val
    }

    fn get_accessory(&self, key: &SlotKey, version: Option<SlotNumber>) -> Option<SlotValue> {
        self.read_value::<Accessory>(key, version)
    }

    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
    ) -> anyhow::Result<(Self::Root, Self::StateUpdate)> {
        let prev_user_root = prev_state_root.namespace_root(ProvableNamespace::User);
        let prev_kernel_root = prev_state_root.namespace_root(ProvableNamespace::Kernel);
        let (user_root, user_state_update) = self
            .compute_state_update_namespace::<DBUserNamespace>(
                state_accesses.user,
                witness,
                jmt::RootHash(prev_user_root),
            )?;

        let (kernel_root, kernel_state_update) = self
            .compute_state_update_namespace::<DBKernelNamespace>(
                state_accesses.kernel,
                witness,
                jmt::RootHash(prev_kernel_root),
            )?;

        Ok((
            StorageRoot::<S>::new(user_root.0, kernel_root.0),
            NamespacedStateUpdate::new(user_state_update, kernel_state_update, Default::default()),
        ))
    }

    fn materialize_changes(self, state_update: Self::StateUpdate) -> Self::ChangeSet {
        let preimages_batch = StateDb::materialize_preimages(
            state_update
                .kernel
                .key_preimages
                .iter()
                .map(|(key_hash, key)| (*key_hash, key.key_ref())),
            state_update
                .user
                .key_preimages
                .iter()
                .map(|(key_hash, key)| (*key_hash, key.key_ref())),
        )
        .expect("collecting preimages must succeed");

        let state_change_set = self
            .db
            .materialize(
                &state_update.kernel.data_to_materialize,
                &state_update.user.data_to_materialize,
                Some(preimages_batch),
            )
            .expect("collecting node batch must succeed");

        let accessory_batch = self.materialize_accessory(&state_update.accessory);

        NativeChangeSet {
            state_change_set,
            accessory_change_set: accessory_batch,
        }
    }

    fn open_proof(
        state_root: Self::Root,
        state_proof: StorageProof<Self::Proof>,
    ) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
        open_merkle_proof(state_root, state_proof)
    }
}

impl<S: MerkleProofSpec> NativeStorage for ProverStorage<S> {
    fn latest_version(&self) -> SlotNumber {
        self.db.get_next_version().saturating_sub(1)
    }

    fn latest_version_unbound(&self) -> SlotNumber {
        // Always return the bound latest version,
        // because detecting an unbound version is error-prone due to kernel/user state split and versions.
        // And JMT works fine with its view of the data.
        self.latest_version()
    }

    fn get_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> Option<SlotValue> {
        self.read_value::<N>(key, version)
    }

    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        self.do_get_leaf::<N>(key, witness, version)
    }

    fn get_with_proof<N: ProvableCompileTimeNamespace>(
        &self,
        key: SlotKey,
        version: Option<SlotNumber>,
    ) -> anyhow::Result<StorageProof<Self::Proof>> {
        let version_to_use = match self.get_version_to_use(version) {
            None => {
                anyhow::bail!(
                    "Proof is not available at version {:?}. Empty storage or future version",
                    version
                )
            }
            Some(v) => v,
        };
        let namespace = N::PROVABLE_NAMESPACE;
        Ok(match namespace {
            ProvableNamespace::User => {
                self.get_with_proof_namespace::<DBUserNamespace>(namespace, key, version_to_use)
            }
            ProvableNamespace::Kernel => {
                self.get_with_proof_namespace::<DBKernelNamespace>(namespace, key, version_to_use)
            }
        })
    }

    fn get_root_hash(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        let version_to_use = match self.get_version_to_use(Some(version)) {
            None => {
                // Mimic error from jmt.
                anyhow::bail!("Root node not found for version {}.", version)
            }
            Some(v) => v,
        };
        self.get_root_hash_unbound(version_to_use)
    }

    fn get_root_hash_unbound(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        let user_root = self
            .get_root_hash_namespace_helper::<DBUserNamespace>(version)
            .context("user namespace")?;
        let kernel_root = self
            .get_root_hash_namespace_helper::<DBKernelNamespace>(version)
            .context("kernel namespace")?;

        Ok(StorageRoot::<S>::new(user_root.0, kernel_root.0))
    }
}
