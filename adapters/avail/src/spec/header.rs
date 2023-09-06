#[cfg(feature = "native")]
use avail_subxt::primitives::Header as SubxtHeader;
use primitive_types::H256;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHeaderTrait;

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
            hash: AvailHash(hash),
            header: Header {
                parent_hash: header.parent_hash,
                number: header.number,
                state_root: header.state_root,
                data_root: header.data_root(),
                extrinsics_root: header.extrinsics_root,
            },
        }
    }

    pub fn data_root(&self) -> AvailHash {
        self.data_root().clone()
    }
}

impl BlockHeaderTrait for AvailHeader {
    type Hash = AvailHash;

    fn prev_hash(&self) -> Self::Hash {
        AvailHash(self.header.parent_hash)
    }

    fn hash(&self) -> Self::Hash {
        self.hash.clone()
    }
}
