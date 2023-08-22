use std::marker::PhantomData;
use std::sync::Arc;

use jmt::{KeyHash, RootHash};
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use zk_cycle_macros::cycle_tracker;

use crate::internal_cache::OrderedReadsAndWrites;
use crate::storage::{StorageKey, StorageProof, StorageValue};
use crate::witness::Witness;
use crate::{MerkleProofSpec, Storage};

#[cfg(all(target_os = "zkvm", feature = "bench"))]
extern crate risc0_zkvm;

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

    type Proof = jmt::proof::SparseMerkleProof<S::Hasher>;

    fn with_config(config: Self::RuntimeConfig) -> Result<Self, anyhow::Error> {
        Ok(Self::new(config))
    }

    fn get(&self, _key: StorageKey, witness: &Self::Witness) -> Option<StorageValue> {
        witness.get_hint()
    }

    fn get_state_root(&self, witness: &Self::Witness) -> anyhow::Result<[u8; 32]> {
        Ok(witness.get_hint())
    }

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn validate_and_commit(
        &self,
        state_accesses: OrderedReadsAndWrites,
        witness: &Self::Witness,
    ) -> Result<[u8; 32], anyhow::Error> {
        // For each value that's been read from the tree, verify the provided smt proof
        for (key, read_value) in state_accesses.ordered_reads {
            let key_hash = KeyHash::with::<S::Hasher>(key.key.as_ref());
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

        let update_proof: jmt::proof::UpdateMerkleProof<S::Hasher> = witness.get_hint();
        let new_root = RootHash(witness.get_hint());

        // Compute the jmt update from the write batch
        let batch = state_accesses
            .ordered_writes
            .into_iter()
            .map(|(key, value)| {
                let key_hash = KeyHash::with::<S::Hasher>(key.key.as_ref());
                (
                    key_hash,
                    value.map(|v| Arc::try_unwrap(v.value).unwrap_or_else(|arc| (*arc).clone())),
                )
            })
            .collect::<Vec<_>>();

        // TODO: Make updates verifiable. Currently, writes don't verify that the provided siblings existed in the old tree
        // because the TreeReader is trusted
        update_proof
            .verify_update(RootHash(self.prev_state_root), new_root, batch)
            .expect("Update proof was invalid! The prover was malicious");

        Ok(new_root.0)
    }

    fn is_empty(&self) -> bool {
        unimplemented!("Needs simplification in JellyfishMerkleTree: https://github.com/Sovereign-Labs/sovereign-sdk/issues/362")
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
