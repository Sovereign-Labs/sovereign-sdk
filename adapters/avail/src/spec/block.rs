use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::services::da::SlotData;

use super::header::AvailHeader;
use super::transaction::AvailBlobTransaction;
use crate::verifier::ChainValidityCondition;

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct AvailBlock {
    pub header: AvailHeader,
    pub transactions: Vec<AvailBlobTransaction>,
}

impl SlotData for AvailBlock {
    type BlockHeader = AvailHeader;
    type Cond = ChainValidityCondition;

    fn hash(&self) -> [u8; 32] {
        *self.header.hash().inner()
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn validity_condition(&self) -> ChainValidityCondition {
        let mut txs_commitment: [u8; 32] = [0u8; 32];

        for tx in &self.transactions {
            txs_commitment = tx.combine_hash(txs_commitment);
        }

        ChainValidityCondition {
            prev_hash: *self.header().prev_hash().inner(),
            block_hash: <Self as SlotData>::hash(self),
            txs_commitment,
        }
    }
}
