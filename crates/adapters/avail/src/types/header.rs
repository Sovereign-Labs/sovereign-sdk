pub use avail_rust_client::prelude::*;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlockHeaderTrait, Time};

use crate::types::{
    hash::AvailHash,
    utils::{KATE_SECONDS_PER_BLOCK, KATE_START_TIME},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CustomAvailHeader {
    pub header: avail_rust_client::AvailHeader,
}

impl BlockHeaderTrait for CustomAvailHeader {
    type Hash = AvailHash;

    fn prev_hash(&self) -> Self::Hash {
        AvailHash(self.header.parent_hash)
    }

    fn hash(&self) -> Self::Hash {
        AvailHash(self.header.parent_hash)
    }

    fn height(&self) -> u64 {
        self.header.number.into()
    }

    fn time(&self) -> Time {
        Time::from_secs(
            KATE_SECONDS_PER_BLOCK
                .saturating_mul(self.header.number as i64)
                .saturating_add(KATE_START_TIME),
        )
    }
}

impl PartialEq for CustomAvailHeader {
    fn eq(&self, other: &Self) -> bool {
        self.header.parent_hash == other.header.parent_hash
            && self.header.number == other.header.number
            && self.header.state_root == other.header.state_root
            && self.header.extrinsics_root == other.header.extrinsics_root
            && self.header.digest == other.header.digest
    }
}
