#[cfg(feature = "native")]
use avail_subxt::primitives::Header as SubxtHeader;
use primitive_types::H256;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHeaderTrait;

const KATE_START_TIME: i64 = 1686066440;
const KATE_SECONDS_PER_BLOCK: i64 = 20;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub parent_hash: H256,
    pub number: u32,
    pub state_root: H256,
    pub extrinsics_root: H256,
    pub data_root: H256,
}

use super::hash::AvailHash;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct AvailHeader {
    hash: AvailHash,

    pub header: Header,
}

#[cfg(feature = "native")]
impl AvailHeader {
    pub fn new(header: SubxtHeader, hash: H256) -> Self {
        Self {
            hash: AvailHash::new(hash),
            header: Header {
                parent_hash: header.parent_hash,
                number: header.number,
                state_root: header.state_root,
                data_root: header.data_root(),
                extrinsics_root: header.extrinsics_root,
            },
        }
    }
}

impl BlockHeaderTrait for AvailHeader {
    type Hash = AvailHash;

    fn prev_hash(&self) -> Self::Hash {
        AvailHash::new(self.header.parent_hash)
    }

    fn hash(&self) -> Self::Hash {
        self.hash.clone()
    }

    fn height(&self) -> u64 {
        self.header.number as u64
    }

    fn time(&self) -> sov_rollup_interface::da::Time {
        sov_rollup_interface::da::Time::from_secs(
            KATE_START_TIME + (self.header.number as i64 * KATE_SECONDS_PER_BLOCK),
        )
    }
}

#[cfg(feature = "native")]
impl
    From<
        subxt::blocks::Block<
            avail_subxt::AvailConfig,
            subxt::OnlineClient<avail_subxt::AvailConfig>,
        >,
    > for AvailHeader
{
    fn from(
        block: subxt::blocks::Block<
            avail_subxt::AvailConfig,
            subxt::OnlineClient<avail_subxt::AvailConfig>,
        >,
    ) -> Self {
        AvailHeader::new(block.header().clone(), block.hash())
    }
}
