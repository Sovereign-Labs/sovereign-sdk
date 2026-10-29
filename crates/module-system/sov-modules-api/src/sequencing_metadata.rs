use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::Bytes;

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

/// Name of the environment variable that overrides the timestamp returned by
/// [`HDTimestamp::default_sequencing_data`] in debug builds.
///
/// Test support only. Exported so that writers (e.g. the `freeze_time` mechanism in
/// `sov-test-utils`) share this definition with the reader instead of duplicating the string.
#[cfg(feature = "native")]
pub const OVERRIDE_HD_TIMESTAMPS_ENV_VAR: &str = "SOV_TEST_OVERRIDE_HD_TIMESTAMPS";

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

    /// Creates the default timestamp used by the preferred sequencer when it attaches
    /// [`SequencingData`] to an accepted transaction.
    #[cfg(feature = "native")]
    pub fn default_sequencing_data() -> Self {
        if cfg!(debug_assertions) {
            match std::env::var(OVERRIDE_HD_TIMESTAMPS_ENV_VAR) {
                // An absent override is the normal case outside tests.
                Err(std::env::VarError::NotPresent) => Self::now(),
                Err(err) => {
                    panic!("{OVERRIDE_HD_TIMESTAMPS_ENV_VAR} is set but unreadable: {err}")
                }
                // An explicitly-set override that doesn't parse is a test-harness bug; falling
                // back to the wall clock would silently un-freeze time and flake tests.
                Ok(timestamp) => Self::from_str(&timestamp).unwrap_or_else(|err| {
                    panic!(
                        "{OVERRIDE_HD_TIMESTAMPS_ENV_VAR} is set to {timestamp:?}, which does \
                         not parse as a nanosecond timestamp: {err}"
                    )
                }),
            }
        } else {
            Self::now()
        }
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

/// Sequencer-provided per-transaction sequencing data.
///
/// This is the decoded form of the `sequencing_data` field of
/// [`FullyBakedTx`](sov_rollup_interface::stf::FullyBakedTx): serialized format-specific
/// sequencing data (interpreted through the runtime's
/// [`SequencingDataFormat`](crate::capabilities::SequencingDataFormat)), plus a timestamp
/// attached by the preferred sequencer.
///
/// The timestamp is SDK-managed: the preferred sequencer attaches it on transaction acceptance
/// and it is always published as-is, while the `data` payload is pruned to the keys accessed
/// during execution.
///
/// # Why a hand-rolled decoder instead of a borsh derive?
///
/// The encoding is equivalent to borsh-serializing an `(Option<u128>, Option<Vec<u8>>)` tuple
/// (pinned by a test; [`Self::encode`] delegates to borsh through an equivalent borrowed
/// tuple). A derived deserializer is deliberately not used: borsh deserializes through an
/// `io::Read`-style interface that cannot borrow from the source buffer, so it would copy the
/// `data` payload (potentially large, e.g. an oracle snapshot) on every decode. [`Self::decode`]
/// instead takes the source as [`Bytes`] and reference-counts the payload out of it zero-copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SequencingData {
    /// Timestamp attached by the preferred sequencer when it accepted the transaction.
    pub timestamp: Option<HDTimestamp>,
    /// Serialized format-specific sequencing data, interpreted by the runtime's
    /// [`SequencingDataFormat`](crate::capabilities::SequencingDataFormat). Private so that
    /// in-execution reads are forced through the access-recording
    /// [`Context::sequencing_data_view`](crate::Context::sequencing_data_view); see
    /// [`Self::unrecorded_data`].
    pub(crate) data: Option<Bytes>,
}

impl SequencingData {
    /// Returns `true` if there is neither a timestamp nor any data.
    pub fn is_empty(&self) -> bool {
        self.timestamp.is_none() && self.data.is_none()
    }

    /// Returns the format-specific data payload WITHOUT recording the access, for test
    /// inspection of published transactions.
    ///
    /// Unrecorded reads are invisible to sequencing data pruning; in-execution reads must go
    /// through [`Context::sequencing_data_view`](crate::Context::sequencing_data_view) instead.
    #[cfg(feature = "test-utils")]
    pub fn unrecorded_data(&self) -> Option<&Bytes> {
        self.data.as_ref()
    }

    /// Serializes the sequencing data.
    ///
    /// The payload is copied exactly once, into an exactly-sized buffer. A zero-copy encode is
    /// not possible: the output must be a single contiguous buffer, since it is embedded in the
    /// transaction wire format.
    pub fn encode(&self) -> Bytes {
        let timestamp_len = 1 + self.timestamp.map_or(0, |_| core::mem::size_of::<u128>());
        let data_len = 1 + self
            .data
            .as_ref()
            .map_or(0, |data| core::mem::size_of::<u32>() + data.len());
        let mut out = Vec::with_capacity(timestamp_len + data_len);
        BorshSerialize::serialize(&(&self.timestamp, self.data.as_deref()), &mut out)
            .expect("sequencing data larger than u32::MAX bytes cannot be encoded");
        out.into()
    }

    /// Deserializes sequencing data, borrowing the `data` payload from `bytes` without copying.
    pub fn decode(bytes: &Bytes) -> anyhow::Result<Self> {
        let mut cursor: &[u8] = bytes;
        let timestamp = <Option<HDTimestamp> as BorshDeserialize>::deserialize(&mut cursor)?;
        let data = match u8::deserialize(&mut cursor)? {
            0 => {
                anyhow::ensure!(
                    cursor.is_empty(),
                    "sequencing data contains {} trailing bytes",
                    cursor.len()
                );
                None
            }
            1 => {
                let len = u32::deserialize(&mut cursor)? as usize;
                anyhow::ensure!(
                    cursor.len() == len,
                    "sequencing data declares {len} data bytes but {} remain",
                    cursor.len()
                );
                Some(bytes.slice_ref(cursor))
            }
            tag => anyhow::bail!("invalid option tag {tag} in sequencing data"),
        };
        Ok(Self { timestamp, data })
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
fn sequencing_data_encode_decode_roundtrip() {
    let sequencing_data = SequencingData {
        timestamp: Some(HDTimestamp(17123456789012345678)),
        data: Some(Bytes::from(vec![0x12, 0x34, 0x56, 0x78])),
    };

    let decoded = SequencingData::decode(&sequencing_data.encode()).unwrap();
    assert_eq!(decoded, sequencing_data);
}

#[test]
fn sequencing_data_encoding_matches_borsh() {
    let sequencing_data = SequencingData {
        timestamp: Some(HDTimestamp(17123456789012345678)),
        data: Some(Bytes::from(vec![0x12, 0x34, 0x56, 0x78])),
    };

    let equivalent_tuple = (
        Some(17123456789012345678u128),
        Some(vec![0x12u8, 0x34, 0x56, 0x78]),
    );
    assert_eq!(
        sequencing_data.encode(),
        borsh::to_vec(&equivalent_tuple).unwrap(),
        "SequencingData must stay wire-compatible with borsh (Option<u128>, Option<Vec<u8>>)"
    );
}

#[test]
fn sequencing_data_decode_rejects_trailing_bytes() {
    let mut encoded = SequencingData {
        timestamp: Some(HDTimestamp(17123456789012345678)),
        data: None,
    }
    .encode()
    .to_vec();
    encoded.push(0xFF);

    SequencingData::decode(&Bytes::from(encoded)).expect_err("decoding must reject trailing bytes");
}
