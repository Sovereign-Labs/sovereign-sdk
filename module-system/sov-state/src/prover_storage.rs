use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use anyhow::ensure;
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

/// A [`Storage`] implementation to be used by the prover in a native execution
/// environment (outside of the zkVM).
pub struct ProverStorage<S: MerkleProofSpec> {
    db: StateDB,
    native_db: NativeDB,
    archival_version: Option<u64>,
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec> Clone for ProverStorage<S> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            native_db: self.native_db.clone(),
            archival_version: self.archival_version,
            _phantom_hasher: Default::default(),
        }
    }
}

impl<S: MerkleProofSpec> ProverStorage<S> {
    /// Creates a new [`ProverStorage`] instance at the specified path, opening
    /// or creating the necessary RocksDB database(s) at the specified path.
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let state_db = StateDB::with_path(&path)?;
        let native_db = NativeDB::with_path(&path)?;

        Ok(Self {
            db: state_db,
            native_db,
            archival_version: None,
            _phantom_hasher: Default::default(),
        })
    }

    pub(crate) fn with_db_handles(db: StateDB, native_db: NativeDB) -> Self {
        Self {
            db,
            native_db,
            archival_version: None,
            _phantom_hasher: Default::default(),
        }
    }

    fn read_value(&self, key: &StorageKey) -> Option<StorageValue> {
        let version = self.archival_version.unwrap_or(self.db.get_next_version());
        match self.db.get_value_option_by_key(version, key.as_ref()) {
            Ok(value) => value.map(Into::into),
            // It is ok to panic here, we assume the db is available and consistent.
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }
}

pub struct ProverStateUpdate {
    pub(crate) node_batch: NodeBatch,
    pub key_preimages: Vec<(KeyHash, CacheKey)>,
    // pub accessory_update: OrderedReadsAndWrites,
}

impl<S: MerkleProofSpec> Storage for ProverStorage<S> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type Root = jmt::RootHash;
    type StateUpdate = ProverStateUpdate;

    fn with_config(config: Self::RuntimeConfig) -> Result<Self, anyhow::Error> {
        Self::with_path(config.path.as_path())
    }

    fn get(&self, key: &StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        let val = self.read_value(key);
        witness.add_hint(val.clone());
        val
    }

    #[cfg(feature = "native")]
    fn get_accessory(&self, key: &StorageKey) -> Option<StorageValue> {
        let version = self
            .archival_version
            .unwrap_or(self.native_db.get_next_version() - 1);
        self.native_db
            .get_value_option(key.as_ref(), Some(version))
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
        self.db
            .put_preimages(
                state_update
                    .key_preimages
                    .iter()
                    .map(|(key_hash, key)| (*key_hash, key.key.as_ref())),
            )
            .expect("Preimage put must succeed");

        self.native_db
            .set_values(
                accessory_writes
                    .ordered_writes
                    .iter()
                    .map(|(k, v_opt)| (k.key.to_vec(), v_opt.as_ref().map(|v| v.value.to_vec()))),
            )
            .expect("native db write must succeed");

        // Write the state values last, since we base our view of what has been touched
        // on state. If the node crashes between the `native_db` update and this update,
        // then the whole `commit` will be re-run later so no data can be lost.
        self.db
            .write_node_batch(&state_update.node_batch)
            .expect("db write must succeed");

        // Finally, update our in-memory view of the current item numbers
        self.db.inc_next_version();
        self.native_db.inc_next_version();
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

    fn set_archival_version(&mut self, version: u64) -> anyhow::Result<()> {
        ensure!(version < self.db.get_next_version(),
            "The storage default read version can not be set to a version greater than the next version");
        self.archival_version = Some(version);
        Ok(())
    }
}
