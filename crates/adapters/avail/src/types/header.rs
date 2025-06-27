pub use avail_rust::prelude::*;
use sov_rollup_interface::da::{BlockHeaderTrait, Time};

use crate::spec::header::AvailHeader;

const KATE_START_TIME: i64 = 1686066440;
const KATE_SECONDS_PER_BLOCK: i64 = 20;

pub struct AvailHeader {
    pub header: avail_rust::AvailHeader,
}

impl BlockHeaderTrait for AvailHeader {
    type Hash = BlockHash;

    fn prev_hash(&self) -> Self::Hash {
        BlockHash(self.header.parent_hash)
    }

    fn hash(&self) -> Self::Hash {
        BlockHash(self.header.hash())
    }

    fn height(&self) -> u64 {
        u64(self.header.number)
    }

    fn time(&self) -> Time {
        Time(
            KATE_SECONDS_PER_BLOCK
                .saturating_mul(self.header.number as i64)
                .saturating_add(KATE_START_TIME),
        )
    }
}
