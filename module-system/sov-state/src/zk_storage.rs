use std::marker::PhantomData;
use std::sync::Arc;

use jmt::{JellyfishMerkleTree, KeyHash, Version};
use sov_rollup_interface::crypto::SimpleHasher;

use crate::internal_cache::OrderedReadsAndWrites;
use crate::storage::{StorageKey, StorageValue};
use crate::witness::{TreeWitnessReader, Witness};
use crate::{MerkleProofSpec, Storage};

pub struct ZkStorage<S: MerkleProofSpec> {
    prev_state_root: [u8; 32],
    _phantom_hasher: PhantomData<S::Hasher>,
}

impl<S: MerkleProofSpec> Clone for ZkStorage<S> {
    fn clone(&self) -> Self {
        Self {
            prev_state_root: self.prev_state_root,
            _phantom_hasher: Default::default(),
        }
    }
}

impl<S: MerkleProofSpec> ZkStorage<S> {
    pub fn new(prev_state_root: [u8; 32]) -> Self {
        Self {
            prev_state_root,
            _phantom_hasher: Default::default(),
        }
    }
}

impl<S: MerkleProofSpec> Storage for ZkStorage<S> {
    type Witness = S::Witness;

    type RuntimeConfig = [u8; 32];

    fn with_config(config: Self::RuntimeConfig) -> Result<Self, anyhow::Error> {
        Ok(Self::new(config))
    }

    fn get(&self, _key: StorageKey, witness: &S::Witness) -> Option<StorageValue> {
        witness.get_hint()
    }

    fn validate_and_commit(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<[u8; 32], anyhow::Error> {
        let latest_version: Version = witness.get_hint();
        let reader = TreeWitnessReader::new(witness);

        // For each value that's been read from the tree, verify the provided smt proof
        for (key, read_value) in state_accesses.ordered_reads {
            let key_hash = KeyHash(S::Hasher::hash(key.key.as_ref()));
            // TODO: Switch to the batch read API once it becomes available
            let proof: jmt::proof::SparseMerkleProof<S::Hasher> = witness.get_hint();
            match read_value {
                Some(val) => proof.verify_existence(
                    jmt::RootHash(self.prev_state_root),
                    key_hash,
                    val.value.as_ref(),
                )?,
                None => proof.verify_nonexistence(jmt::RootHash(self.prev_state_root), key_hash)?,
            }
        }

        // Compute the jmt update from the write batch
        let batch = state_accesses
            .ordered_writes
            .into_iter()
            .map(|(key, value)| {
                let key_hash = KeyHash(S::Hasher::hash(key.key.as_ref()));
                (
                    key_hash,
                    value.map(|v| Arc::try_unwrap(v.value).unwrap_or_else(|arc| (*arc).clone())),
                )
            });

        let next_version = latest_version + 1;
        // TODO: Make updates verifiable. Currently, writes don't verify that the provided siblings existed in the old tree
        // because the TreeReader is trusted
        let jmt = JellyfishMerkleTree::<_, S::Hasher>::new(&reader);

        let (new_root, _tree_update) = jmt
            .put_value_set(batch, next_version)
            .expect("JMT update must succeed");

        Ok(new_root.0)
    }

    fn is_empty(&self) -> bool {
        unimplemented!("Needs simplification in JellyfishMerkleTree: https://github.com/Sovereign-Labs/sovereign-sdk/issues/362")
    }
}
