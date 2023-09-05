use std::fs;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use jmt::storage::{NodeBatch, TreeWriter};
use jmt::{JellyfishMerkleTree, KeyHash, RootHash, Version};
use sov_db::native_db::NativeDB;
use sov_db::state_db::StateDB;

use crate::config::Config;
use crate::internal_cache::OrderedReadsAndWrites;
use crate::storage::{NativeStorage, StorageKey, StorageProof, StorageValue};
use crate::tree_db::TreeReadLogger;
use crate::witness::Witness;
use crate::{MerkleProofSpec, Storage};

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
    pub fn with_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let state_db = StateDB::with_path(&path)?;
        let native_db = NativeDB::with_path(&path)?;

        Ok(Self {
            db: state_db,
            native_db,
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

    fn get_root_hash(&self, version: Version) -> Result<RootHash, anyhow::Error> {
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

    fn get_state_root(&self, _witness: &Self::Witness) -> anyhow::Result<[u8; 32]> {
        self.get_root_hash(self.db.get_next_version() - 1)
            .map(|root| root.0)
    }

    fn compute_state_update(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<([u8; 32], Self::StateUpdate), anyhow::Error> {
        let latest_version = self.db.get_next_version() - 1;
        witness.add_hint(latest_version);

        let read_logger = TreeReadLogger::with_db_and_witness(self.db.clone(), witness);
        let untracked_jmt = JellyfishMerkleTree::<_, S::Hasher>::new(&self.db);

        // Handle empty untracked_jmt
        if untracked_jmt
            .get_root_hash_option(latest_version)?
            .is_none()
        {
            assert_eq!(latest_version, 0);
            let empty_batch = Vec::default().into_iter();
            let (_, tree_update) = untracked_jmt
                .put_value_set(empty_batch, latest_version)
                .expect("JMT update must succeed");

            self.db
                .write_node_batch(&tree_update.node_batch)
                .expect("db write must succeed");
        }

        // For each value that's been read from the tree, read it from the logged JMT to populate hints
        for (key, read_value) in state_accesses.ordered_reads {
            let key_hash = KeyHash::with::<S::Hasher>(key.key.as_ref());
            // TODO: Switch to the batch read API once it becomes available
            let (result, proof) = untracked_jmt.get_with_proof(key_hash, latest_version)?;
            if result.as_ref() != read_value.as_ref().map(|f| f.value.as_ref()) {
                anyhow::bail!("Bug! Incorrect value read from jmt");
            }
            witness.add_hint(proof);
        }

        let tracked_jmt = JellyfishMerkleTree::<_, S::Hasher>::new(&read_logger);
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

        let (new_root, tree_update) = tracked_jmt
            .put_value_set(batch, next_version)
            .expect("JMT update must succeed");

        Ok((new_root.0, tree_update.node_batch))
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
        state_root: [u8; 32],
        state_proof: StorageProof<Self::Proof>,
    ) -> Result<(StorageKey, Option<StorageValue>), anyhow::Error> {
        let StorageProof { key, value, proof } = state_proof;
        let key_hash = KeyHash::with::<S::Hasher>(key.as_ref());

        proof.verify(
            jmt::RootHash(state_root),
            key_hash,
            value.as_ref().map(|v| v.value()),
        )?;
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

pub fn delete_storage(path: impl AsRef<Path>) {
    fs::remove_dir_all(&path)
        .or_else(|_| fs::remove_file(&path))
        .unwrap();
}

#[cfg(test)]
mod test {
    use jmt::Version;

    use super::*;
    use crate::{DefaultStorageSpec, StateReaderAndWriter, WorkingSet};

    #[derive(Clone)]
    struct TestCase {
        key: StorageKey,
        value: StorageValue,
        version: Version,
    }

    fn create_tests() -> Vec<TestCase> {
        vec![
            TestCase {
                key: StorageKey::from("key_0"),
                value: StorageValue::from("value_0"),
                version: 1,
            },
            TestCase {
                key: StorageKey::from("key_1"),
                value: StorageValue::from("value_1"),
                version: 2,
            },
            TestCase {
                key: StorageKey::from("key_2"),
                value: StorageValue::from("value_2"),
                version: 3,
            },
        ]
    }

    #[test]
    fn test_jmt_storage() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();
        let tests = create_tests();
        {
            for test in tests.clone() {
                let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
                let mut storage = WorkingSet::new(prover_storage.clone());
                assert_eq!(prover_storage.db.get_next_version(), test.version);

                storage.set(&test.key, test.value.clone());
                let (cache, witness) = storage.checkpoint().freeze();
                prover_storage
                    .validate_and_commit(cache, &witness)
                    .expect("storage is valid");

                assert_eq!(test.value, prover_storage.get(&test.key, &witness).unwrap());
                assert_eq!(prover_storage.db.get_next_version(), test.version + 1)
            }
        }

        {
            let storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert_eq!(storage.db.get_next_version(), (tests.len() + 1) as u64);
            for test in tests {
                assert_eq!(
                    test.value,
                    storage.get(&test.key, &Default::default()).unwrap()
                );
            }
        }
    }

    #[test]
    fn test_restart_lifecycle() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path();
        {
            let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert!(prover_storage.is_empty());
        }

        let key = StorageKey::from("some_key");
        let value = StorageValue::from("some_value");
        // First restart
        {
            let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert!(prover_storage.is_empty());
            let mut storage = WorkingSet::new(prover_storage.clone());
            storage.set(&key, value.clone());
            let (cache, witness) = storage.checkpoint().freeze();
            prover_storage
                .validate_and_commit(cache, &witness)
                .expect("storage is valid");
        }

        // Correctly restart from disk
        {
            let prover_storage = ProverStorage::<DefaultStorageSpec>::with_path(path).unwrap();
            assert!(!prover_storage.is_empty());
            assert_eq!(
                value,
                prover_storage.get(&key, &Default::default()).unwrap()
            );
        }
    }
}
