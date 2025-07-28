mod address;

use std::fmt::{Debug, Formatter};

pub use address::{MockAddress, MOCK_SEQUENCER_DA_ADDRESS};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::{
    BlockHashTrait, BlockHeaderTrait, CountedBufReader, DaProof, RelevantBlobs, RelevantProofs,
    Time,
};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;
use sov_rollup_interface::Bytes;

use crate::utils::hash_to_array;

/// Serialized aggregated proof.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct Proof(pub(crate) Vec<u8>);

/// A mock hash digest.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    BorshDeserialize,
    BorshSerialize,
    derive_more::From,
    derive_more::Into,
    UniversalWallet,
)]
pub struct MockHash(pub [u8; 32]);

impl Debug for MockHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", HexHash::new(self.0))
    }
}

impl core::fmt::Display for MockHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", HexHash::new(self.0))
    }
}

impl core::str::FromStr for MockHash {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let inner = HexHash::from_str(s)?;
        Ok(MockHash(inner.0))
    }
}

impl AsRef<[u8]> for MockHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<Vec<u8>> for MockHash {
    type Error = anyhow::Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let hash: [u8; 32] = value.try_into().map_err(|e: Vec<u8>| {
            anyhow::anyhow!("Vec<u8> should have length 32: but it has {}", e.len())
        })?;
        Ok(MockHash(hash))
    }
}

impl BlockHashTrait for MockHash {}

/// A mock block header used for testing.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, derive_more::Display)]
#[display("{:?}", self)]
pub struct MockBlockHeader {
    /// The height of this block.
    pub height: u64,
    /// The hash of the previous block.
    pub prev_hash: MockHash,
    /// The hash of this block.
    pub hash: MockHash,
    /// The time at which this block was created.
    pub time: Time,
}

impl MockBlockHeader {
    /// Generates [`MockBlockHeader`] with given height & time, where hashes are derived from height.
    /// Can be used in tests, where a header of the following blocks will be consistent.
    pub fn new(height: u64, time: Time) -> MockBlockHeader {
        let prev_hash = u64_to_bytes(height);
        let hash = u64_to_bytes(height + 1);
        MockBlockHeader {
            height,
            hash: MockHash(hash),
            prev_hash: MockHash(prev_hash),
            time,
        }
    }

    /// Generates [`MockBlockHeader`] with given height, where hashes are derived from height.
    /// Can be used in tests, where a header of the following blocks will be consistent.
    pub fn from_height(height: u64) -> MockBlockHeader {
        Self::new(height, Time::now())
    }
}

impl Default for MockBlockHeader {
    fn default() -> Self {
        MockBlockHeader::from_height(0)
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
    pub(crate) hash: MockHash,
    pub(crate) blob: CountedBufReader<Bytes>,
}

impl MockBlob {
    /// Creates a new mock blob with the given data, claiming to have been published by the provided address.
    pub fn new(tx_blob: Vec<u8>, address: MockAddress, hash: [u8; 32]) -> Self {
        Self {
            address,
            blob: CountedBufReader::new(Bytes::from(tx_blob)),
            hash: MockHash(hash),
        }
    }

    /// Build new blob, but calculates hash from input data
    pub fn new_with_hash(blob: Vec<u8>, address: MockAddress) -> Self {
        let data_hash = hash_to_array(&blob).to_vec();
        let blob_hash = MockHash(hash_to_array(&data_hash));
        Self {
            address,
            blob: CountedBufReader::new(Bytes::from(blob)),
            hash: blob_hash,
        }
    }

    /// Creates blob of transactions.
    pub fn advance(&mut self) {
        self.blob.advance(self.blob.total_len());
    }
}

/// A mock block type used for testing.
#[derive(Serialize, Deserialize, Default, PartialEq, Debug, Clone)]
pub struct MockBlock {
    /// The header of this block.
    pub header: MockBlockHeader,
    /// Rollup's batch namespace.
    pub batch_blobs: Vec<MockBlob>,
    /// Rollup's proof namespace.
    pub proof_blobs: Vec<MockBlob>,
}

#[cfg(feature = "native")]
impl sov_rollup_interface::node::da::SlotData for MockBlock {
    type BlockHeader = MockBlockHeader;

    fn hash(&self) -> [u8; 32] {
        self.header.hash.0
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn timestamp(&self) -> Time {
        self.header.time.clone()
    }
}

impl MockBlock {
    /// Creates empty block, which is following of the current
    pub fn next_mock(&self) -> MockBlock {
        Self::default_at_height(self.header.height + 1)
    }

    /// Creates an empty block at the given height.
    pub fn default_at_height(height: u64) -> Self {
        MockBlock {
            header: MockBlockHeader::from_height(height),
            ..Default::default()
        }
    }

    /// Creates [`RelevantBlobs`] data from this block.
    /// Where all batches and proofs are relevant.
    pub fn as_relevant_blobs(&self) -> RelevantBlobs<MockBlob> {
        RelevantBlobs {
            proof_blobs: self.proof_blobs.clone(),
            batch_blobs: self.batch_blobs.clone(),
        }
    }

    /// Creates [`RelevantProofs`] with default values for inclusion and completeness proofs.
    pub fn get_relevant_proofs(&self) -> RelevantProofs<[u8; 32], ()> {
        RelevantProofs {
            batch: DaProof {
                inclusion_proof: Default::default(),
                completeness_proof: Default::default(),
            },
            proof: DaProof {
                inclusion_proof: Default::default(),
                completeness_proof: Default::default(),
            },
        }
    }
}

fn u64_to_bytes(value: u64) -> [u8; 32] {
    let value = value.to_be_bytes();
    let mut result = [0u8; 32];
    result[..value.len()].copy_from_slice(&value);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_to_string() {
        let header = MockBlockHeader {
            prev_hash: MockHash([1; 32]),
            hash: MockHash([2; 32]),
            height: 1,
            time: Time::from_secs(1672531200),
        };

        let expected = "MockBlockHeader { height: 1, prev_hash: 0x0101010101010101010101010101010101010101010101010101010101010101, hash: 0x0202020202020202020202020202020202020202020202020202020202020202, time: Time { millis: 1672531200000 } }";

        assert_eq!(expected, header.to_string());
    }
}
