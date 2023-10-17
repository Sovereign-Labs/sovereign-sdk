use std::marker::PhantomData;
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard};

use jmt::storage::NodeBatch;
use jmt::{JellyfishMerkleTree, KeyHash};
use sov_db::native_db::NativeDB;
use sov_db::state_db::StateDB;

use crate::config::Config;
use crate::storage::{QuerySnapshotLayers, SnapshotId, StorageKey, StorageProof, StorageValue};
use crate::{MerkleProofSpec, OrderedReadsAndWrites, Storage, Witness};

pub struct ReadOnlyLock<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> ReadOnlyLock<T> {
    #[allow(dead_code)]
    pub fn new(lock: Arc<RwLock<T>>) -> Self {
        Self { lock }
    }

    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        self.lock.read()
    }
}

impl<T> Clone for ReadOnlyLock<T> {
    fn clone(&self) -> Self {
        Self {
            lock: self.lock.clone(),
        }
    }
}

/// A storage implementation that uses a [`QuerySnapshotLayers`] before checking [`StateDB`].
/// Other naming variants:
/// SnapshotCascadeProverStorage: "Cascade" implies that there's a sequence or chain of events or checks. This name gives an indication of the step-by-step checking process through different snapshot layers.
/// LayeredProverStorage: The word "sequential" implies an ordered or step-by-step process. This could represent the fact that the storage checks layers in sequence.
pub struct HierarchicalProverStorage<S: MerkleProofSpec, Q: QuerySnapshotLayers> {
    id: SnapshotId,
    db: StateDB,
    native_db: NativeDB,
    parent: ReadOnlyLock<Q>,
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec, Q: QuerySnapshotLayers> HierarchicalProverStorage<S, Q> {
    #[allow(dead_code)]
    pub fn new_from_db(
        id: SnapshotId,
        state_db: StateDB,
        native_db: NativeDB,
        manager: ReadOnlyLock<Q>,
    ) -> Self {
        Self {
            id,
            db: state_db,
            native_db,
            parent: manager,
            _phantom_hasher: Default::default(),
        }
    }

    fn read_value(&self, key: &StorageKey) -> Option<StorageValue> {
        match self
            .db
            .get_value_option_by_key(self.db.get_next_version(), key.as_ref())
        {
            Ok(value) => value.map(Into::into),
            // It is ok to panic here, we assume the db is available and consistent.
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }
}

impl<S: MerkleProofSpec, Q: QuerySnapshotLayers> Clone for HierarchicalProverStorage<S, Q> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            db: self.db.clone(),
            native_db: self.native_db.clone(),
            parent: self.parent.clone(),
            _phantom_hasher: Default::default(),
        }
    }
}

impl<Q: QuerySnapshotLayers, S: MerkleProofSpec> Storage for HierarchicalProverStorage<S, Q> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type Root = jmt::RootHash;
    type StateUpdate = NodeBatch;

    fn with_config(_config: Self::RuntimeConfig) -> Result<Self, anyhow::Error> {
        todo!("Won't be implemented. ForkManager will be creating its storage instead")
    }

    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        let parent_snapshot_manager = self.parent.read().unwrap();
        let val = match parent_snapshot_manager.fetch_value(&self.id, key) {
            Some(val) => Some(val),
            None => self.read_value(key),
        };

        witness.add_hint(val.clone());
        val
    }

    #[cfg(feature = "native")]
    fn get_accessory(&self, key: &StorageKey) -> Option<StorageValue> {
        let parent_snapshot_manager = self.parent.read().unwrap();
        match parent_snapshot_manager.fetch_accessory_value(&self.id, key) {
            Some(val) => Some(val),
            None => self
                .native_db
                .get_value_option(key.as_ref())
                .unwrap()
                .map(Into::into),
        }
    }

    fn compute_state_update(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<(Self::Root, Self::StateUpdate), anyhow::Error> {
        // THIS IS INCREMENT...
        let latest_version = self.db.get_next_version() - 1;
        let jmt = JellyfishMerkleTree::<_, S::Hasher>::new(&self.db);

        assert!(
            jmt.get_root_hash_option(latest_version)?.is_some(),
            "underlying db was not setup"
        );

        let prev_root = jmt
            .get_root_hash(latest_version)
            .expect("Previous root hash was just populated");
        witness.add_hint(prev_root.0);

        let batch = state_accesses
            .ordered_writes
            .into_iter()
            .map(|(key, value)| {
                let key_hash = KeyHash::with::<S::Hasher>(key.key.as_ref());
                // NOTE: SKIP PRE_IMAGE
                (
                    key_hash,
                    value.map(|v| Arc::try_unwrap(v.value).unwrap_or_else(|arc| (*arc).clone())),
                )
            });

        // TODO: IS THIS WRITE TO DB??
        let next_version = self.db.get_next_version();

        // TODO: IS THIS WRITE TO DB??
        let (new_root, update_proof, tree_update) = jmt
            .put_value_set_with_proof(batch, next_version)
            .expect("JMT update must succeed");

        witness.add_hint(update_proof);
        witness.add_hint(new_root.0);

        Ok((new_root, tree_update.node_batch))
    }

    fn commit(&self, _node_batch: &Self::StateUpdate, _accessory_update: &OrderedReadsAndWrites) {
        todo!("Won't be implemented")
    }

    fn open_proof(
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), anyhow::Error> {
        let StorageProof { key, value, proof } = proof;
        let key_hash = KeyHash::with::<S::Hasher>(key.as_ref());

        proof.verify(state_root, key_hash, value.as_ref().map(|v| v.value()))?;
        Ok((key, value))
    }

    fn is_empty(&self) -> bool {
        self.db.get_next_version() <= 1
    }
}
