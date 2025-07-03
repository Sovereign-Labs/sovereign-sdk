use avail_rust::H256;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::{da::Time, node::da::SlotData};

use crate::types::{data::AvailData, header::AvailHeader};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvailBlock {
    pub header: AvailHeader,
    pub block_hash: H256,
    pub timestamp: Time,
    pub batch_blobs: Vec<AvailData>,
    pub proof_blobs: Vec<AvailData>,
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
        self.timestamp.clone()
    }
}

impl AvailBlock {
    #[cfg(feature = "native")]
    pub fn new(
        header: AvailHeader,
        block_hash: H256,
        timestamp: Time,
        batch_blobs: Vec<AvailData>,
        proof_blobs: Vec<AvailData>,
    ) -> Self {
        Self {
            header,
            block_hash,
            timestamp,
            batch_blobs,
            proof_blobs,
        }
    }
}
