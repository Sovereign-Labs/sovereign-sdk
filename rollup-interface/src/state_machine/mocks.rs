//! Defines mock instantiations of many important traits, which are useful
//! for testing, fuzzing, and benchmarking.
use std::fmt::Display;
use std::io::Write;

use anyhow::{ensure, Error};
use borsh::{BorshDeserialize, BorshSerialize};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::da::{BlobTransactionTrait, BlockHashTrait, BlockHeaderTrait, CountedBufReader, DaSpec};
use crate::services::da::SlotData;
use crate::zk::{Matches, Zkvm};
use crate::AddressTrait;

/// A mock commitment to a particular zkVM program.
#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct MockCodeCommitment(pub [u8; 32]);

impl Matches<MockCodeCommitment> for MockCodeCommitment {
    fn matches(&self, other: &MockCodeCommitment) -> bool {
        self.0 == other.0
    }
}

/// A mock proof generated by a zkVM.
#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct MockProof<'a> {
    /// The ID of the program this proof might be valid for.
    pub program_id: MockCodeCommitment,
    /// Whether the proof is valid.
    pub is_valid: bool,
    /// The tamper-proof outputs of the proof.
    pub log: &'a [u8],
}

impl<'a> MockProof<'a> {
    /// Serializes a proof into a writer.
    pub fn encode(&self, mut writer: impl Write) {
        writer.write_all(&self.program_id.0).unwrap();
        let is_valid_byte = if self.is_valid { 1 } else { 0 };
        writer.write_all(&[is_valid_byte]).unwrap();
        writer.write_all(self.log).unwrap();
    }

    /// Serializes a proof into a vector.
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        self.encode(&mut encoded);
        encoded
    }

    /// Tries to deserialize a proof from a byte slice.
    pub fn decode(input: &'a [u8]) -> Result<Self, anyhow::Error> {
        ensure!(input.len() >= 33, "Input is too short");
        let program_id = MockCodeCommitment(input[0..32].try_into().unwrap());
        let is_valid = input[32] == 1;
        let log = &input[33..];
        Ok(Self {
            program_id,
            is_valid,
            log,
        })
    }
}

/// A mock implementing the zkVM trait.
pub struct MockZkvm;

impl Zkvm for MockZkvm {
    type CodeCommitment = MockCodeCommitment;

    type Error = anyhow::Error;

    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        let proof = MockProof::decode(serialized_proof)?;
        anyhow::ensure!(
            proof.program_id.matches(code_commitment),
            "Proof failed to verify against requested code commitment"
        );
        anyhow::ensure!(proof.is_valid, "Proof is not valid");
        Ok(proof.log)
    }
}

#[test]
fn test_mock_proof_roundtrip() {
    let proof = MockProof {
        program_id: MockCodeCommitment([1; 32]),
        is_valid: true,
        log: &[2; 50],
    };

    let mut encoded = Vec::new();
    proof.encode(&mut encoded);

    let decoded = MockProof::decode(&encoded).unwrap();
    assert_eq!(proof, decoded);
}

/// A mock address type used for testing. Internally, this type is standard 32 byte array.
#[derive(Debug, PartialEq, Clone, Eq, Copy, serde::Serialize, serde::Deserialize)]
pub struct MockAddress {
    addr: [u8; 32],
}

impl core::str::FromStr for MockAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let addr = hex::decode(s)?;
        if addr.len() != 32 {
            return Err(anyhow::anyhow!("Invalid address length"));
        }

        let mut array = [0; 32];
        array.copy_from_slice(&addr);
        Ok(MockAddress { addr: array })
    }
}

impl<'a> TryFrom<&'a [u8]> for MockAddress {
    type Error = Error;

    fn try_from(addr: &'a [u8]) -> Result<Self, Self::Error> {
        if addr.len() != 32 {
            anyhow::bail!("Address must be 32 bytes long");
        }
        let mut addr_bytes = [0u8; 32];
        addr_bytes.copy_from_slice(addr);
        Ok(Self { addr: addr_bytes })
    }
}

impl AsRef<[u8]> for MockAddress {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl From<[u8; 32]> for MockAddress {
    fn from(addr: [u8; 32]) -> Self {
        MockAddress { addr }
    }
}

impl Display for MockAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.addr)
    }
}

impl std::hash::Hash for MockAddress {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.addr.hash(state);
    }
}

impl AddressTrait for MockAddress {}

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
pub struct TestBlob<Address> {
    address: Address,
    hash: [u8; 32],
    data: CountedBufReader<Bytes>,
}

impl<Address: AddressTrait> BlobTransactionTrait for TestBlob<Address> {
    type Data = Bytes;
    type Address = Address;

    fn sender(&self) -> Self::Address {
        self.address.clone()
    }

    fn hash(&self) -> [u8; 32] {
        self.hash
    }

    fn data_mut(&mut self) -> &mut CountedBufReader<Self::Data> {
        &mut self.data
    }

    fn data(&self) -> &CountedBufReader<Self::Data> {
        &self.data
    }
}

impl<Address: AddressTrait> TestBlob<Address> {
    /// Creates a new mock blob with the given data, claiming to have been published by the provided address.
    pub fn new(data: Vec<u8>, address: Address, hash: [u8; 32]) -> Self {
        Self {
            address,
            data: CountedBufReader::new(bytes::Bytes::from(data)),
            hash,
        }
    }
}

/// A mock hash digest.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TestHash(pub [u8; 32]);

impl AsRef<[u8]> for TestHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl BlockHashTrait for TestHash {}

/// A mock block header used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone)]
pub struct TestBlockHeader {
    /// The hash of the previous block.
    pub prev_hash: TestHash,
}

impl BlockHeaderTrait for TestBlockHeader {
    type Hash = TestHash;

    fn prev_hash(&self) -> Self::Hash {
        self.prev_hash
    }

    fn hash(&self) -> Self::Hash {
        TestHash(sha2::Sha256::digest(self.prev_hash.0).into())
    }
}

/// A mock block type used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone)]
pub struct TestBlock {
    /// The hash of this block.
    pub curr_hash: [u8; 32],
    /// The header of this block.
    pub header: TestBlockHeader,
    /// The height of this block
    pub height: u64,
}

impl SlotData for TestBlock {
    type BlockHeader = TestBlockHeader;
    fn hash(&self) -> [u8; 32] {
        self.curr_hash
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }
}

/// A [`DaSpec`] suitable for testing.
pub struct MockDaSpec;

impl DaSpec for MockDaSpec {
    type SlotHash = TestHash;
    type BlockHeader = TestBlockHeader;
    type BlobTransaction = TestBlob<MockAddress>;
    type InclusionMultiProof = [u8; 32];
    type CompletenessProof = ();
    type ChainParams = ();
}
