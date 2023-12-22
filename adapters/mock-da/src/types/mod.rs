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
pub struct MockHash(pub [u8; 32]);

impl Debug for MockHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl core::fmt::Display for MockHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct MockBlockHeader {
    /// The hash of the previous block.
    pub prev_hash: MockHash,
    /// The hash of this block.
    pub hash: MockHash,
    /// The height of this block
    pub height: u64,
    /// The time at which this block was created
    pub time: Time,
}

impl MockBlockHeader {
    /// Generates [`MockBlockHeader`] with given height, where hashes are derived from height
    /// Can be used in tests, where header of following blocks will be consistent
    pub fn from_height(height: u64) -> MockBlockHeader {
        let prev_hash = u64_to_bytes(height);
        let hash = u64_to_bytes(height + 1);
        MockBlockHeader {
            prev_hash: MockHash(prev_hash),
            hash: MockHash(hash),
            height,
            time: Time::now(),
        }
    }
}

impl Default for MockBlockHeader {
    fn default() -> Self {
        MockBlockHeader::from_height(0)
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
        self.time.clone()
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
    // Data for the aggregated ZK proof.
    pub(crate) zk_proofs_data: Vec<u8>,
}

impl MockBlob {
    /// Creates a new mock blob with the given data, claiming to have been published by the provided address.
    pub fn new(data: Vec<u8>, address: MockAddress, hash: [u8; 32]) -> Self {
        Self {
            address,
            data: CountedBufReader::new(Bytes::from(data)),
            zk_proofs_data: Default::default(),
            hash,
        }
    }

    /// Creates a new mock blob with the given data and an aggretated zkp proof, claiming to have been published by the provided address.
    pub fn new_with_zkp_proof(
        data: Vec<u8>,
        zk_proofs_data: Vec<u8>,
        address: MockAddress,
        hash: [u8; 32],
    ) -> Self {
        Self {
            address,
            hash,
            data: CountedBufReader::new(Bytes::from(data)),
            zk_proofs_data,
        }
    }
}

/// A mock block type used for testing.
#[derive(Serialize, Deserialize, Default, PartialEq, Debug, Clone)]
pub struct MockBlock {
    /// The header of this block.
    pub header: MockBlockHeader,
    /// Validity condition
    pub validity_cond: MockValidityCond,
    /// Blobs
    pub blobs: Vec<MockBlob>,
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

impl MockBlock {
    /// Creates empty block, which is following of the current
    pub fn next_mock(&self) -> MockBlock {
        let mut next_block = MockBlock::default();
        let h = self.header.height + 1;
        next_block.header = MockBlockHeader::from_height(h);
        next_block
    }
}

fn u64_to_bytes(value: u64) -> [u8; 32] {
    let value = value.to_be_bytes();
    let mut result = [0u8; 32];
    result[..value.len()].copy_from_slice(&value);
    result
}
