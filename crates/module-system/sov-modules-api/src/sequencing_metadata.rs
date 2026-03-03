use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use borsh::{BorshDeserialize, BorshSerialize};

use crate::capabilities::SequencingDataTrait;

/// High definition execution timestamp, in nanoseconds since the unix epoch.
#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    BorshSerialize,
    BorshDeserialize,
    serde::Serialize,
)]
#[serde(transparent)]
pub struct HDTimestamp(u128);

impl SequencingDataTrait for HDTimestamp {
    fn get_maybe_timestamp(self) -> Option<HDTimestamp> {
        Some(self)
    }
}

impl HDTimestamp {
    /// Creates timestamp with current time
    pub fn now() -> Self {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(ts)
    }

    /// Returns the timestamp as nanoseconds since the unix epoch.
    pub const fn as_nanos(&self) -> u128 {
        self.0
    }
}

impl std::fmt::Display for HDTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for HDTimestamp {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, std::num::ParseIntError> {
        Ok(Self(s.parse::<u128>()?))
    }
}

#[test]
fn test_hd_timestamp_display_roundtrip() {
    let nanos = 17123456789012345678;
    let timestamp = HDTimestamp(nanos);
    let timestamp = timestamp.to_string();
    assert_eq!(timestamp, nanos.to_string());
    let timestamp = HDTimestamp::from_str(&timestamp).unwrap();
    assert_eq!(timestamp.as_nanos(), nanos);
}
