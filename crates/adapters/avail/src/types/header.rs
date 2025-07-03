pub use avail_rust::prelude::*;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlockHeaderTrait, Time};

use crate::types::hash::AvailHash;

const KATE_START_TIME: i64 = 1686066440;
const KATE_SECONDS_PER_BLOCK: i64 = 20;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AvailHeader {
    pub header: avail_rust::AvailHeader,
}

impl BlockHeaderTrait for AvailHeader {
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
