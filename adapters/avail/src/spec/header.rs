use super::hash::AvailHash;
use avail_subxt::primitives::Header;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::traits::{BlockHeaderTrait, CanonicalHash};
use subxt::utils::H256;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct AvailHeader {
    hash: AvailHash,

    pub header: Header,
}

impl AvailHeader {
    pub fn new(header: Header, hash: H256) -> Self {
        Self {
            hash: AvailHash(hash),
            header,
        }
    }
}

impl BlockHeaderTrait for AvailHeader {
    type Hash = AvailHash;

    fn prev_hash(&self) -> Self::Hash {
        AvailHash(self.header.parent_hash)
    }
}

impl CanonicalHash for AvailHeader {
    type Output = AvailHash;

    fn hash(&self) -> Self::Output {
        self.hash.clone()
    }
}
