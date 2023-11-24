use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, ensure, Error};
use jmt::storage::{NodeBatch, TreeWriter};
use jmt::{JellyfishMerkleTree, KeyHash, Version};
use sov_db::native_db::NativeDB;
use sov_db::state_db::StateDB;
use sov_modules_core::{
    CacheKey, NativeStorage, OrderedReadsAndWrites, Storage, StorageKey, StorageProof,
    StorageValue, Witness,
};

use crate::config::Config;
use crate::MerkleProofSpec;

/// placeholder
pub enum Storages<S: MerkleProofSpec> {
    /// placeholder
    Prover(ProverStorage<S>),
    /// placeholder
    Archival(ArchivalStorage<S>),
}

impl<S: MerkleProofSpec> Clone for Storages<S> {
    fn clone(&self) -> Self {
        match self {
            Storages::Prover(prover_storage) => Storages::Prover(prover_storage.clone()),
            Storages::Archival(archival_storage) => Storages::Archival(archival_storage.clone()),
        }
    }
}

impl<S: MerkleProofSpec> Storages<S> {
    /// placeholder
    pub fn get_archival_storage(&self, version: u64) -> anyhow::Result<Storages<S>> {
        match self {
            Storages::Prover(prover_storage) => Ok(Storages::Archival(
                prover_storage.get_archival_storage(version)?,
            )),
            Storages::Archival(archival_storage) => Ok(Storages::Archival(
                archival_storage.clone().set_archival_version(version)?,
            )),
        }
    }
}

impl<S: MerkleProofSpec> Storage for Storages<S> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type Root = jmt::RootHash;
    type StateUpdate = ProverStateUpdate;

    fn with_config(config: Self::RuntimeConfig) -> anyhow::Result<Self> {
        Ok(Storages::Prover(ProverStorage::with_config(config)?))
    }

    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        match self {
            Storages::Prover(prover_storage) => prover_storage.get(key, witness),
            Storages::Archival(archival_storage) => archival_storage.get(key, witness),
        }
    }

    #[cfg(feature = "native")]
    fn get_accessory(&self, key: &StorageKey) -> Option<StorageValue> {
        match self {
            Storages::Prover(prover_storage) => prover_storage.get_accessory(key),
            Storages::Archival(archival_storage) => archival_storage.get_accessory(key),
        }
    }

    fn compute_state_update(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<(Self::Root, Self::StateUpdate), Error> {
        match self {
            Storages::Prover(prover_storage) => {
                prover_storage.compute_state_update(state_accesses, witness)
            }
            Storages::Archival(archival_storage) => {
                archival_storage.compute_state_update(state_accesses, witness)
            }
        }
    }

    fn commit(&self, node_batch: &Self::StateUpdate, accessory_update: &OrderedReadsAndWrites) {
        match self {
            Storages::Prover(prover_storage) => prover_storage.commit(node_batch, accessory_update),
            Storages::Archival(archival_storage) => {
                archival_storage.commit(node_batch, accessory_update)
            }
        }
    }

    fn open_proof(
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), Error> {
        ProverStorage::<S>::open_proof(state_root, proof)
    }

    fn is_empty(&self) -> bool {
        match self {
            Storages::Prover(prover_storage) => prover_storage.is_empty(),
            Storages::Archival(archival_storage) => archival_storage.is_empty(),
        }
    }
}

impl<S: MerkleProofSpec> NativeStorage for Storages<S> {
    fn get_with_proof(&self, key: StorageKey) -> StorageProof<Self::Proof> {
        match self {
            Storages::Prover(prover_storage) => prover_storage.get_with_proof(key),
            Storages::Archival(archival_storage) => archival_storage.get_with_proof(key),
        }
    }

    fn get_root_hash(&self, version: Version) -> Result<Self::Root, Error> {
        match self {
            Storages::Prover(prover_storage) => prover_storage.get_root_hash(version),
            Storages::Archival(archival_storage) => archival_storage.get_root_hash(version),
        }
    }
}

/// A [`Storage`] implementation to be used by the prover in a native execution
/// environment (outside of the zkVM).
pub struct ProverStorage<S: MerkleProofSpec> {
    db: StateDB,
    native_db: NativeDB,
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec> Clone for ProverStorage<S> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            native_db: self.native_db.clone(),
            _phantom_hasher: Default::default(),
        }
    }
}

impl<S: MerkleProofSpec> ProverStorage<S> {
    /// Creates a new [`ProverStorage`] instance at the specified path, opening
    /// or creating the necessary RocksDB database(s) at the specified path.
    pub fn with_path(path: impl AsRef<Path>) -> Result<Storages<S>, anyhow::Error> {
        let state_db = StateDB::with_path(&path)?;
        let native_db = NativeDB::with_path(&path)?;

        Ok(Storages::Prover(Self {
            db: state_db,
            native_db,
            _phantom_hasher: Default::default(),
        }))
    }

    /// Creates a new [`ProverStorage`] instance at the specified path, opening
    /// or creating the necessary RocksDB database(s) at the specified path.
    pub fn with_path_prover(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let state_db = StateDB::with_path(&path)?;
        let native_db = NativeDB::with_path(&path)?;

        Ok(Self {
            db: state_db,
            native_db,
            _phantom_hasher: Default::default(),
        })
    }

    pub(crate) fn with_db_handles(db: StateDB, native_db: NativeDB) -> Self {
        Self {
            db,
            native_db,
            _phantom_hasher: Default::default(),
        }
    }

    fn read_value(&self, key: &StorageKey) -> Option<StorageValue> {
        let version = self.db.get_next_version();
        match self.db.get_value_option_by_key(version, key.as_ref()) {
            Ok(value) => value.map(Into::into),
            // It is ok to panic here, we assume the db is available and consistent.
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }

    /// placeholder
    pub fn get_archival_storage(&self, version: Version) -> anyhow::Result<ArchivalStorage<S>> {
        ArchivalStorage::new(self.clone(), version)
    }
}

pub struct ProverStateUpdate {
    pub(crate) node_batch: NodeBatch,
    pub key_preimages: Vec<(KeyHash, CacheKey)>,
}

impl<S: MerkleProofSpec> Storage for ProverStorage<S> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type Root = jmt::RootHash;
    type StateUpdate = ProverStateUpdate;

    fn with_config(config: Self::RuntimeConfig) -> Result<Self, anyhow::Error> {
        match Self::with_path(config.path.as_path()) {
            Ok(storages) => match storages {
                Storages::Prover(prover_storage) => Ok(prover_storage),
                _ => bail!("Creating storage failed"),
            },
            Err(_) => bail!("Creating storage failed"),
        }
    }

    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        let val = self.read_value(key);
        witness.add_hint(val.clone());
        val
    }

    #[cfg(feature = "native")]
    fn get_accessory(&self, key: &StorageKey) -> Option<StorageValue> {
        let version = self.db.get_next_version() - 1;
        self.native_db
            .get_value_option(key.as_ref(), version)
            .unwrap()
            .map(Into::into)
    }

    fn compute_state_update(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<(Self::Root, Self::StateUpdate), anyhow::Error> {
        let latest_version = self.db.get_next_version() - 1;
        let jmt = JellyfishMerkleTree::<_, S::Hasher>::new(&self.db);

        // Handle empty jmt
        // TODO: Fix this before introducing snapshots!
        if jmt.get_root_hash_option(latest_version)?.is_none() {
            assert_eq!(latest_version, 0);
            let empty_batch = Vec::default().into_iter();
            let (_, tree_update) = jmt
                .put_value_set(empty_batch, latest_version)
                .expect("JMT update must succeed");

            self.db
                .write_node_batch(&tree_update.node_batch)
                .expect("db write must succeed");
        }
        let prev_root = jmt
            .get_root_hash(latest_version)
            .expect("Previous root hash was just populated");
        witness.add_hint(prev_root.0);

        // For each value that's been read from the tree, read it from the logged JMT to populate hints
        for (key, read_value) in state_accesses.ordered_reads {
            let key_hash = KeyHash::with::<S::Hasher>(key.key.as_ref());
            // TODO: Switch to the batch read API once it becomes available
            let (result, proof) = jmt.get_with_proof(key_hash, latest_version)?;
            if result.as_ref() != read_value.as_ref().map(|f| f.value.as_ref()) {
                anyhow::bail!("Bug! Incorrect value read from jmt");
            }
            witness.add_hint(proof);
        }

        let mut key_preimages = Vec::with_capacity(state_accesses.ordered_writes.len());

        // Compute the jmt update from the write batch
        let batch = state_accesses
            .ordered_writes
            .into_iter()
            .map(|(key, value)| {
                let key_hash = KeyHash::with::<S::Hasher>(key.key.as_ref());
                key_preimages.push((key_hash, key));
                (
                    key_hash,
                    value.map(|v| Arc::try_unwrap(v.value).unwrap_or_else(|arc| (*arc).clone())),
                )
            });

        let next_version = self.db.get_next_version();

        let (new_root, update_proof, tree_update) = jmt
            .put_value_set_with_proof(batch, next_version)
            .expect("JMT update must succeed");

        witness.add_hint(update_proof);
        witness.add_hint(new_root.0);

        let state_update = ProverStateUpdate {
            node_batch: tree_update.node_batch,
            key_preimages,
        };

        Ok((new_root, state_update))
    }

    fn commit(&self, state_update: &Self::StateUpdate, accessory_writes: &OrderedReadsAndWrites) {
        let latest_version = self.db.get_next_version() - 1;
        for (key_hash, key) in state_update.key_preimages.iter() {
            // Clone should be cheap
            self.db
                .put_preimage(*key_hash, key.key.as_ref())
                .expect("preimage must succeed");
        }

        self.db
            .write_node_batch(&state_update.node_batch)
            .expect("db write must succeed");

        self.native_db
            .set_values(
                accessory_writes
                    .ordered_writes
                    .iter()
                    .map(|(k, v_opt)| (k.key.to_vec(), v_opt.as_ref().map(|v| v.value.to_vec()))),
                latest_version,
            )
            .expect("native db write must succeed");

        self.db.inc_next_version();
    }

    fn open_proof(
        state_root: Self::Root,
        state_proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), anyhow::Error> {
        let StorageProof { key, value, proof } = state_proof;
        let key_hash = KeyHash::with::<S::Hasher>(key.as_ref());

        proof.verify(state_root, key_hash, value.as_ref().map(|v| v.value()))?;
        Ok((key, value))
    }

    // Based on assumption `validate_and_commit` increments version.
    fn is_empty(&self) -> bool {
        self.db.get_next_version() <= 1
    }
}

impl<S: MerkleProofSpec> NativeStorage for ProverStorage<S> {
    fn get_with_proof(&self, key: StorageKey) -> StorageProof<Self::Proof> {
        let merkle = JellyfishMerkleTree::<StateDB, S::Hasher>::new(&self.db);
        let (val_opt, proof) = merkle
            .get_with_proof(
                KeyHash::with::<S::Hasher>(key.as_ref()),
                self.db.get_next_version() - 1,
            )
            .unwrap();
        StorageProof {
            key,
            value: val_opt.map(StorageValue::from),
            proof,
        }
    }

    fn get_root_hash(&self, version: Version) -> Result<jmt::RootHash, anyhow::Error> {
        let temp_merkle: JellyfishMerkleTree<'_, StateDB, S::Hasher> =
            JellyfishMerkleTree::new(&self.db);
        temp_merkle.get_root_hash(version)
    }
}

/// placeholder
pub struct ArchivalStorage<S: MerkleProofSpec> {
    prover_storage: ProverStorage<S>,
    archival_version: Version,
}

impl<S: MerkleProofSpec> ArchivalStorage<S> {
    /// placeholder
    pub fn new(prover_storage: ProverStorage<S>, version: Version) -> anyhow::Result<Self> {
        ensure!(version < prover_storage.db.get_next_version(),
            "The storage default read version can not be set to a version greater than the next version");
        Ok(Self {
            prover_storage,
            archival_version: version,
        })
    }

    /// placeholder
    pub fn set_archival_version(mut self, version: Version) -> anyhow::Result<Self> {
        ensure!(version < self.prover_storage.db.get_next_version(),
            "The storage default read version can not be set to a version greater than the next version");
        self.archival_version = version;
        Ok(self)
    }
}

impl<S: MerkleProofSpec> Clone for ArchivalStorage<S> {
    fn clone(&self) -> Self {
        Self {
            prover_storage: self.prover_storage.clone(),
            archival_version: self.archival_version,
        }
    }
}

impl<S: MerkleProofSpec> Storage for ArchivalStorage<S> {
    type Witness = <ProverStorage<S> as Storage>::Witness;
    type RuntimeConfig = <ProverStorage<S> as Storage>::RuntimeConfig;
    type Proof = <ProverStorage<S> as Storage>::Proof;
    type Root = <ProverStorage<S> as Storage>::Root;
    type StateUpdate = <ProverStorage<S> as Storage>::StateUpdate;

    fn with_config(config: Self::RuntimeConfig) -> anyhow::Result<Self> {
        ProverStorage::<S>::with_config(config).map(|prover_storage| {
            let archival_version = prover_storage.db.get_next_version();
            ArchivalStorage {
                prover_storage,
                archival_version,
            }
        })
    }

    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        let val = match self
            .prover_storage
            .db
            .get_value_option_by_key(self.archival_version, key.as_ref())
        {
            Ok(value) => value.map(Into::into),
            Err(e) => panic!("Unable to read value from db: {e}"),
        };
        witness.add_hint(val.clone());
        val
    }

    #[cfg(feature = "native")]
    fn get_accessory(&self, key: &StorageKey) -> Option<StorageValue> {
        self.prover_storage
            .native_db
            .get_value_option(key.as_ref(), self.archival_version)
            .unwrap()
            .map(Into::into)
    }

    fn compute_state_update(
        &self,
        _state_accesses: OrderedReadsAndWrites,
        _witness: &Self::Witness,
    ) -> Result<(Self::Root, Self::StateUpdate), Error> {
        bail!("Archival storage cannot update");
    }

    fn commit(&self, _node_batch: &Self::StateUpdate, _accessory_update: &OrderedReadsAndWrites) {}

    fn open_proof(
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), Error> {
        ProverStorage::<S>::open_proof(state_root, proof)
    }

    fn is_empty(&self) -> bool {
        self.prover_storage.is_empty()
    }
}

impl<S: MerkleProofSpec> NativeStorage for ArchivalStorage<S> {
    fn get_with_proof(&self, key: StorageKey) -> StorageProof<Self::Proof> {
        let merkle = JellyfishMerkleTree::<StateDB, S::Hasher>::new(&self.prover_storage.db);
        let (val_opt, proof) = merkle
            .get_with_proof(
                KeyHash::with::<S::Hasher>(key.as_ref()),
                self.archival_version,
            )
            .unwrap();
        StorageProof {
            key,
            value: val_opt.map(StorageValue::from),
            proof,
        }
    }

    fn get_root_hash(&self, version: Version) -> Result<Self::Root, Error> {
        let temp_merkle: JellyfishMerkleTree<'_, StateDB, S::Hasher> =
            JellyfishMerkleTree::new(&self.prover_storage.db);
        temp_merkle.get_root_hash(version)
    }
}
