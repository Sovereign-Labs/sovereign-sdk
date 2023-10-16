use std::marker::PhantomData;
use std::sync::{LockResult, RwLock, RwLockReadGuard};

use anyhow::Error;
use jmt::storage::NodeBatch;
use sha2::digest::const_oid::Arc;
use sov_db::native_db::NativeDB;
use sov_db::state_db::StateDB;

use crate::config::Config;
use crate::storage::{QuerySnapshotLayers, SnapshotId, StorageKey, StorageProof, StorageValue};
use crate::{MerkleProofSpec, OrderedReadsAndWrites, Storage};

#[derive(Clone)]
pub struct ReadOnlyLock<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> ReadOnlyLock<T> {
    pub fn new(lock: Arc<RwLock<T>>) -> Self {
        Self { lock }
    }

    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        self.lock.read()
    }
}

#[derive(Clone)]
pub struct TreeQuery<S: MerkleProofSpec, Q: QuerySnapshotLayers> {
    id: SnapshotId,
    db: StateDB,
    native_db: NativeDB,
    manager: ReadOnlyLock<Q>,
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<Q: QuerySnapshotLayers, S: MerkleProofSpec> Storage for TreeQuery<S, Q> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type Root = jmt::RootHash;
    type StateUpdate = NodeBatch;

    fn with_config(config: Self::RuntimeConfig) -> Result<Self, Error> {
        todo!()
    }

    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        todo!()
    }

    fn compute_state_update(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<(Self::Root, Self::StateUpdate), Error> {
        todo!()
    }

    fn commit(&self, node_batch: &Self::StateUpdate, accessory_update: &OrderedReadsAndWrites) {
        todo!()
    }

    fn open_proof(
        &self,
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), Error> {
        todo!()
    }

    fn is_empty(&self) -> bool {
        todo!()
    }
}
