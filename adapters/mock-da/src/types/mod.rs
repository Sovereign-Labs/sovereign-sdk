mod address;

use std::fmt::{Debug, Formatter};
use std::hash::Hasher;

pub use address::{MockAddress, MOCK_SEQUENCER_DA_ADDRESS};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlockHashTrait, BlockHeaderTrait, CountedBufReader, Time};
use sov_rollup_interface::services::da::SlotData;
use sov_rollup_interface::Bytes;

use crate::validity_condition::MockValidityCond;

/// A mock hash digest.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    BorshDeserialize,
    BorshSerialize,
)]
pub struct MockHash([u8; 32]);

impl Debug for MockHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl AsRef<[u8]> for MockHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; 32]> for MockHash {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl From<MockHash> for [u8; 32] {
    fn from(value: MockHash) -> Self {
        value.0
    }
}

impl std::hash::Hash for MockHash {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(&self.0);
        state.finish();
    }
}

impl BlockHashTrait for MockHash {}

/// A mock block header used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone, Copy)]
pub struct MockBlockHeader {
    /// The hash of the previous block.
    pub prev_hash: MockHash,
    /// The hash of this block.
    pub hash: MockHash,
    /// The height of this block
    pub height: u64,
}

impl Default for MockBlockHeader {
    fn default() -> Self {
        Self {
            prev_hash: MockHash([0u8; 32]),
            hash: MockHash([1u8; 32]),
            height: 0,
        }
    }
}

impl std::fmt::Display for MockBlockHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MockBlockHeader {{ height: {}, prev_hash: {}, next_hash: {} }}",
            self.height,
            hex::encode(self.prev_hash),
            hex::encode(self.hash)
        )
    }
}

impl BlockHeaderTrait for MockBlockHeader {
    type Hash = MockHash;

    fn prev_hash(&self) -> Self::Hash {
        self.prev_hash
    }

    fn hash(&self) -> Self::Hash {
        self.hash
    }

    fn height(&self) -> u64 {
        self.height
    }

    fn time(&self) -> Time {
        Time::now()
    }
}

/// The configuration for mock da
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MockDaConfig {
    /// The address to use to "submit" blobs on the mock da layer
    pub sender_address: MockAddress,
}

#[derive(Clone, Default)]
/// DaVerifier used in tests.
pub struct MockDaVerifier {}

#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
/// A mock BlobTransaction from a DA layer used for testing.
pub struct MockBlob {
    pub(crate) address: MockAddress,
    pub(crate) hash: [u8; 32],
    /// Actual data from the blob. Public for testing purposes.
    pub data: CountedBufReader<Bytes>,
}

impl MockBlob {
    /// Creates a new mock blob with the given data, claiming to have been published by the provided address.
    pub fn new(data: Vec<u8>, address: MockAddress, hash: [u8; 32]) -> Self {
        Self {
            address,
            data: CountedBufReader::new(Bytes::from(data)),
            hash,
        }
    }
}

/// A mock block type used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone)]
pub struct MockBlock {
    /// The header of this block.
    pub header: MockBlockHeader,
    /// Validity condition
    pub validity_cond: MockValidityCond,
    /// Blobs
    pub blobs: Vec<MockBlob>,
}

impl Default for MockBlock {
    fn default() -> Self {
        Self {
            header: MockBlockHeader {
                prev_hash: [0; 32].into(),
                hash: [1; 32].into(),
                height: 0,
            },
            validity_cond: Default::default(),
            blobs: Default::default(),
        }
    }
}

impl SlotData for MockBlock {
    type BlockHeader = MockBlockHeader;
    type Cond = MockValidityCond;

    fn hash(&self) -> [u8; 32] {
        self.header.hash.0
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn validity_condition(&self) -> MockValidityCond {
        self.validity_cond
    }
}
