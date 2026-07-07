use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use borsh::{BorshDeserialize, BorshSerialize};

use crate::capabilities::SequencingDataFormat;

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

impl SequencingDataFormat for HDTimestamp {
    type Key = ();
    type Value = HDTimestamp;

    fn get(bytes: &[u8], _key: &Self::Key) -> anyhow::Result<Option<Self::Value>> {
        Ok(Some(Self::try_from_slice(bytes)?))
    }

    #[cfg(feature = "native")]
    fn prune_to_used_keys(
        bytes: sov_rollup_interface::Bytes,
        used_keys: &std::collections::BTreeSet<Self::Key>,
    ) -> anyhow::Result<Option<sov_rollup_interface::Bytes>> {
        Ok(used_keys.contains(&()).then_some(bytes))
    }
}

#[cfg(feature = "native")]
const OVERRIDE_HD_TIMESTAMPS_ENV_VAR: &str = "SOV_TEST_OVERRIDE_HD_TIMESTAMPS";

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

    /// Creates the default timestamp metadata used by the preferred sequencer.
    #[cfg(feature = "native")]
    pub fn default_sequencing_data() -> Self {
        if cfg!(debug_assertions) {
            let Ok(timestamp) = std::env::var(OVERRIDE_HD_TIMESTAMPS_ENV_VAR) else {
                return Self::now();
            };
            Self::from_str(&timestamp).unwrap_or_else(|_| Self::now())
        } else {
            Self::now()
        }
    }

    /// Creates byte-compatible default timestamp metadata for a fully baked transaction.
    #[cfg(feature = "native")]
    pub fn default_sequencing_data_bytes() -> sov_rollup_interface::Bytes {
        borsh::to_vec(&Self::default_sequencing_data())
            .expect("HDTimestamp serialization is infallible")
            .into()
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

#[test]
fn hd_timestamp_sequencing_format_is_byte_compatible() {
    let nanos = 17123456789012345678;
    let timestamp = HDTimestamp(nanos);
    let bytes = borsh::to_vec(&timestamp).unwrap();

    assert_eq!(bytes, nanos.to_le_bytes());
    assert_eq!(
        HDTimestamp::get(&bytes, &()).unwrap().unwrap().as_nanos(),
        nanos
    );
}

#[cfg(feature = "native")]
#[test]
fn hd_timestamp_prunes_only_when_accessed() {
    let timestamp = HDTimestamp(17123456789012345678);
    let bytes: sov_rollup_interface::Bytes = borsh::to_vec(&timestamp).unwrap().into();

    assert_eq!(
        HDTimestamp::prune_to_used_keys(bytes.clone(), &std::collections::BTreeSet::from([()]))
            .unwrap(),
        Some(bytes.clone())
    );
    assert_eq!(
        HDTimestamp::prune_to_used_keys(bytes, &std::collections::BTreeSet::new()).unwrap(),
        None
    );
}
