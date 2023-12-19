use std::marker::PhantomData;
use std::sync::Arc;

use jmt::storage::{NodeBatch, TreeWriter};
use jmt::{JellyfishMerkleTree, KeyHash, Version};
use sov_db::native_db::NativeDB;
use sov_db::schema::{QueryManager, ReadOnlyDbSnapshot};
use sov_db::state_db::StateDB;
use sov_modules_core::{
    CacheKey, NativeStorage, OrderedReadsAndWrites, Storage, StorageKey, StorageProof,
    StorageValue, Witness,
};

use crate::config::Config;
use crate::MerkleProofSpec;

/// A [`Storage`] implementation to be used by the prover in a native execution
/// environment (outside of the zkVM).
pub struct ProverStorage<S: MerkleProofSpec, Q> {
    db: StateDB<Q>,
    native_db: NativeDB<Q>,
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec, Q> Clone for ProverStorage<S, Q> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            native_db: self.native_db.clone(),
            _phantom_hasher: Default::default(),
        }
    }
}

impl<S: MerkleProofSpec, Q> ProverStorage<S, Q> {
    /// Creates a new [`ProverStorage`] instance from specified db handles
    pub fn with_db_handles(db: StateDB<Q>, native_db: NativeDB<Q>) -> Self {
        Self {
            db,
            native_db,
            _phantom_hasher: Default::default(),
        }
    }

    /// Converts it to pair of readonly [`ReadOnlyDbSnapshot`]s
    /// First is from [`StateDB`]
    /// Second is from [`NativeDB`]
    pub fn freeze(self) -> anyhow::Result<(ReadOnlyDbSnapshot, ReadOnlyDbSnapshot)> {
        let ProverStorage { db, native_db, .. } = self;
        let state_db_snapshot = db.freeze()?;
        let native_db_snapshot = native_db.freeze()?;
        Ok((state_db_snapshot, native_db_snapshot))
    }
}

impl<S: MerkleProofSpec, Q: QueryManager> ProverStorage<S, Q> {
    fn read_value(&self, key: &StorageKey, version: Option<Version>) -> Option<StorageValue> {
        let version_to_use = version.unwrap_or_else(|| self.db.get_next_version());
        match self
            .db
            .get_value_option_by_key(version_to_use, key.as_ref())
        {
            Ok(value) => value.map(Into::into),
            // It is ok to panic here, we assume the db is available and consistent.
            Err(e) => panic!("Unable to read value from db: {e}"),
        }
    }
}

pub struct ProverStateUpdate {
    pub(crate) node_batch: NodeBatch,
    pub key_preimages: Vec<(KeyHash, CacheKey)>,
}

impl<S: MerkleProofSpec, Q: QueryManager> Storage for ProverStorage<S, Q> {
    type Witness = S::Witness;
    type RuntimeConfig = Config;
    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;
    type Root = jmt::RootHash;
    type StateUpdate = ProverStateUpdate;

    fn get(
        &self,
        key: &StorageKey,
        version: Option<Version>,
        witness: &Self::Witness,
    ) -> Option<StorageValue> {
        let val = self.read_value(key, version);
        witness.add_hint(val.clone());
        val
    }

    #[cfg(feature = "native")]
    fn get_accessory(&self, key: &StorageKey, version: Option<Version>) -> Option<StorageValue> {
        let version_to_use = version.unwrap_or_else(|| self.db.get_next_version() - 1);
        self.native_db
            .get_value_option(key.as_ref(), version_to_use)
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
                latest_version,
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

impl<S: MerkleProofSpec, Q: QueryManager> NativeStorage for ProverStorage<S, Q> {
    fn get_with_proof(&self, key: StorageKey) -> StorageProof<Self::Proof> {
        let merkle = JellyfishMerkleTree::<StateDB<Q>, S::Hasher>::new(&self.db);
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

    fn get_root_hash(&self, version: Version) -> anyhow::Result<jmt::RootHash> {
        let temp_merkle: JellyfishMerkleTree<'_, StateDB<Q>, S::Hasher> =
            JellyfishMerkleTree::new(&self.db);
        temp_merkle.get_root_hash(version)
    }
}
