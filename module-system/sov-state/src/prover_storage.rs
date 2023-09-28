use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use jmt::storage::{NodeBatch, TreeWriter};
use jmt::{JellyfishMerkleTree, KeyHash, RootHash, Version};
use sov_db::native_db::NativeDB;
use sov_db::state_db::StateDB;

use crate::config::Config;
use crate::internal_cache::OrderedReadsAndWrites;
use crate::storage::{NativeStorage, Storage, StorageKey, StorageProof, StorageValue};
use crate::witness::Witness;
use crate::MerkleProofSpec;

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
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let state_db = StateDB::with_path(&path)?;
        let native_db = NativeDB::with_path(&path)?;

        Ok(Self {
            db: state_db,
            native_db,
            _phantom_hasher: Default::default(),
        })
    }

    /// Returns the underlying [`StateDB`] instance.
    pub fn db(&self) -> &StateDB {
        &self.db
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

    /// Get the root hash of the tree at the requested version
    pub fn get_root_hash(&self, version: Version) -> Result<RootHash, anyhow::Error> {
        let temp_merkle: JellyfishMerkleTree<'_, StateDB, S::Hasher> =
            JellyfishMerkleTree::new(&self.db);
        temp_merkle.get_root_hash(version)
    }
}

impl<S: MerkleProofSpec> Storage for ProverStorage<S> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type StateUpdate = NodeBatch;
    type Root = jmt::RootHash;

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
        self.native_db
            .get_value_option(key.as_ref())
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

        // Compute the jmt update from the write batch
        let batch = state_accesses
            .ordered_writes
            .into_iter()
            .map(|(key, value)| {
                let key_hash = KeyHash::with::<S::Hasher>(key.key.as_ref());
                self.db
                    .put_preimage(key_hash, key.key.as_ref())
                    .expect("preimage must succeed");
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

        Ok((new_root, tree_update.node_batch))
    }

    fn commit(&self, node_batch: &Self::StateUpdate, accessory_writes: &OrderedReadsAndWrites) {
        self.db
            .write_node_batch(node_batch)
            .expect("db write must succeed");

        self.native_db
            .set_values(
                accessory_writes
                    .ordered_writes
                    .iter()
                    .map(|(k, v_opt)| (k.key.to_vec(), v_opt.as_ref().map(|v| v.value.to_vec())))
                    .collect(),
            )
            .expect("native db write must succeed");

        self.db.inc_next_version();
    }

    // Based on assumption `validate_and_commit` increments version.
    fn is_empty(&self) -> bool {
        self.db.get_next_version() <= 1
    }

    fn open_proof(
        &self,
        state_root: Self::Root,
        state_proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), anyhow::Error> {
        let StorageProof { key, value, proof } = state_proof;
        let key_hash = KeyHash::with::<S::Hasher>(key.as_ref());

        proof.verify(state_root, key_hash, value.as_ref().map(|v| v.value()))?;
        Ok((key, value))
    }
}

impl<S: MerkleProofSpec> NativeStorage for ProverStorage<S> {
    fn get_with_proof(
        &self,
        key: StorageKey,
        _witness: &Self::Witness,
    ) -> StorageProof<Self::Proof> {
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
}
