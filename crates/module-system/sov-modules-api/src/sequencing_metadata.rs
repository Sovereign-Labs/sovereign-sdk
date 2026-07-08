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

    /// Creates the default timestamp used by the preferred sequencer when it attaches
    /// [`SequencingData`] to an accepted transaction.
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
/// # Why a hand-rolled codec instead of a borsh derive?
///
/// The encoding is equivalent to borsh-serializing an `(Option<u128>, Option<Vec<u8>>)` tuple
/// (pinned by a test). A derive is deliberately not used: borsh deserializes through an
/// `io::Read`-style interface that cannot borrow from the source buffer, so it would copy the
/// `data` payload (potentially large, e.g. an oracle snapshot) on every decode. [`Self::decode`]
/// instead takes the source as [`Bytes`] and reference-counts the payload out of it zero-copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SequencingData {
    /// Timestamp attached by the preferred sequencer when it accepted the transaction.
    pub timestamp: Option<HDTimestamp>,
    /// Serialized format-specific sequencing data, interpreted by the runtime's
    /// [`SequencingDataFormat`](crate::capabilities::SequencingDataFormat).
    pub data: Option<Bytes>,
}

impl SequencingData {
    /// Returns `true` if there is neither a timestamp nor any data.
    pub fn is_empty(&self) -> bool {
        self.timestamp.is_none() && self.data.is_none()
    }

    /// Serializes the sequencing data.
    pub fn encode(&self) -> Bytes {
        let mut out = borsh::to_vec(&self.timestamp)
            .expect("serializing an Option<HDTimestamp> is infallible");
        match &self.data {
            None => out.push(0),
            Some(data) => {
                out.push(1);
                let len = u32::try_from(data.len())
                    .expect("sequencing data larger than u32::MAX bytes cannot be encoded");
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(data);
            }
        }
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
