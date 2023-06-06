use super::{header::AvailHeader, transaction::AvailBlobTransaction};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::{services::da::SlotData, traits::CanonicalHash};

#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub struct AvailBlock {
    pub header: AvailHeader,
    pub transactions: Vec<AvailBlobTransaction>,
}

impl SlotData for AvailBlock {
    type BlockHeader = AvailHeader;

    fn hash(&self) -> [u8; 32] {
        self.header.hash().0 .0
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }
}
