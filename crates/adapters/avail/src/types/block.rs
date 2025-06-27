use avail_rust::BlockHash;
// Adjust to your crate structure// Or your local path
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::{da::Time, node::da::SlotData};

use crate::types::header::AvailHeader; // The trait to implement

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AvailBlock {
    pub header: AvailHeader,
    pub block_hash: BlockHash,
    pub timestamp: Time,
}

impl SlotData for AvailBlock {
    type BlockHeader = AvailHeader;

    fn hash(&self) -> [u8; 32] {
        self.block_hash.into()
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn timestamp(&self) -> Time {
        self.timestamp
    }
}
