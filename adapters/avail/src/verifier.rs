use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec, DaVerifier};
use sov_rollup_interface::zk::ValidityCondition;
use thiserror::Error;

use crate::spec::DaLayerSpec;

#[derive(Error, Debug)]
pub enum ValidityConditionError {
    #[error("conditions for validity can only be combined if the blocks are consecutive")]
    BlocksNotConsecutive,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Copy, BorshDeserialize, BorshSerialize,
)]
/// A validity condition expressing that a chain of DA layer blocks is contiguous and canonical
pub struct ChainValidityCondition {
    pub prev_hash: [u8; 32],
    pub block_hash: [u8; 32],
    //Chained or batch txs commitment.
    pub txs_commitment: [u8; 32],
}

impl ValidityCondition for ChainValidityCondition {
    type Error = ValidityConditionError;

    fn combine<SimpleHasher>(&self, rhs: Self) -> Result<Self, Self::Error> {
        let mut combined_hashes: Vec<u8> = Vec::with_capacity(64);
        combined_hashes.extend_from_slice(self.txs_commitment.as_ref());
        combined_hashes.extend_from_slice(rhs.txs_commitment.as_ref());

        let combined_root = sp_core_hashing::blake2_256(&combined_hashes);

        if self.block_hash != rhs.prev_hash {
            return Err(ValidityConditionError::BlocksNotConsecutive);
        }

        Ok(Self {
            prev_hash: rhs.prev_hash,
            block_hash: rhs.block_hash,
            txs_commitment: combined_root,
        })
    }
}

pub struct Verifier;

impl DaVerifier for Verifier {
    type Spec = DaLayerSpec;

    type Error = ValidityConditionError;

    // Verify that the given list of blob transactions is complete and correct.
    // NOTE: Function return unit since application client already verifies application data.
    fn verify_relevant_tx_list(
        &self,
        block_header: &<Self::Spec as DaSpec>::BlockHeader,
        txs: &[<Self::Spec as DaSpec>::BlobTransaction],
        _inclusion_proof: <Self::Spec as DaSpec>::InclusionMultiProof,
        _completeness_proof: <Self::Spec as DaSpec>::CompletenessProof,
    ) -> Result<<Self::Spec as DaSpec>::ValidityCondition, Self::Error> {
        let mut txs_commitment: [u8; 32] = [0u8; 32];

        for tx in txs {
            txs_commitment = tx.combine_hash(txs_commitment);
        }

        let validity_condition = ChainValidityCondition {
            prev_hash: *block_header.prev_hash().inner(),
            block_hash: *block_header.hash().inner(),
            txs_commitment,
        };

        Ok(validity_condition)
    }

    fn new(_params: <Self::Spec as DaSpec>::ChainParams) -> Self {
        Verifier {}
    }
}
