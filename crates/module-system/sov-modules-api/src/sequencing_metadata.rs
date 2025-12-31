use std::time::{SystemTime, UNIX_EPOCH};

use borsh::{BorshDeserialize, BorshSerialize};

/// High definition execution timestamp
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, BorshSerialize, BorshDeserialize)]
pub struct HDTimestamp(u128);

impl HDTimestamp {
    /// Creates timestamp with current time
    pub fn now() -> Self {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(ts)
    }
}
