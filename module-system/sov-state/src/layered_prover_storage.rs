use std::marker::PhantomData;
use std::path::Path;
use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard};

use anyhow::Error;
use jmt::storage::NodeBatch;
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

pub struct TreeQuery<S: MerkleProofSpec, Q: QuerySnapshotLayers> {
    id: SnapshotId,
    db: StateDB,
    native_db: NativeDB,
    manager: ReadOnlyLock<Q>,
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec, Q: QuerySnapshotLayers> TreeQuery<S, Q> {
    #[allow(dead_code)]
    pub fn new(
        id: SnapshotId,
        path: impl AsRef<Path>,
        manager: ReadOnlyLock<Q>,
    ) -> Result<Self, anyhow::Error> {
        let state_db = StateDB::with_path(&path)?;
        let native_db = NativeDB::with_path(&path)?;

        Ok(Self {
            id,
            db: state_db,
            native_db,
            manager,
            _phantom_hasher: Default::default(),
        })
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

impl<S: MerkleProofSpec, Q: QuerySnapshotLayers> Clone for TreeQuery<S, Q> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            db: self.db.clone(),
            native_db: self.native_db.clone(),
            manager: self.manager.clone(),
            _phantom_hasher: Default::default(),
        }
    }
}

impl<Q: QuerySnapshotLayers, S: MerkleProofSpec> Storage for TreeQuery<S, Q> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type Root = jmt::RootHash;
    type StateUpdate = NodeBatch;

    fn with_config(_config: Self::RuntimeConfig) -> Result<Self, Error> {
        todo!()
    }

    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        let manager = self.manager.read().unwrap();
        let val = match manager.fetch_value(&self.id, key) {
            Some(val) => Some(val),
            None => self.read_value(key),
        };

        witness.add_hint(val.clone());
        val
    }

    fn compute_state_update(
        &self,
        _state_accesses: OrderedReadsAndWrites,
        _witness: &Self::Witness,
    ) -> Result<(Self::Root, Self::StateUpdate), Error> {
        todo!()
    }

    fn commit(&self, _node_batch: &Self::StateUpdate, _accessory_update: &OrderedReadsAndWrites) {
        todo!("Won't be implemented")
    }

    fn open_proof(
        _state_root: Self::Root,
        _proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), Error> {
        todo!()
    }

    fn is_empty(&self) -> bool {
        todo!()
    }
}
