use std::io::Write;

use anyhow::ensure;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::{
    da::BlobTransactionTrait,
    traits::AddressTrait,
    zk::traits::{Matches, Zkvm},
};

#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct MockCodeCommitment(pub [u8; 32]);

impl Matches<MockCodeCommitment> for MockCodeCommitment {
    fn matches(&self, other: &MockCodeCommitment) -> bool {
        self.0 == other.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct MockProof<'a> {
    pub program_id: MockCodeCommitment,
    pub is_valid: bool,
    pub log: &'a [u8],
}

impl<'a> MockProof<'a> {
    pub fn encode(&self, mut writer: impl Write) {
        writer.write_all(&self.program_id.0).unwrap();
        let is_valid_byte = if self.is_valid { 1 } else { 0 };
        writer.write_all(&[is_valid_byte]).unwrap();
        writer.write_all(self.log).unwrap();
    }

    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        self.encode(&mut encoded);
        encoded
    }

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

#[derive(
    Debug,
    Clone,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct TestBlob<Address> {
    address: Address,
    data: Vec<u8>,
}

impl<Address: AddressTrait> BlobTransactionTrait for TestBlob<Address> {
    type Data = std::io::Cursor<Vec<u8>>;

    type Address = Address;

    fn sender(&self) -> Self::Address {
        self.address.clone()
    }

    fn data(&self) -> Self::Data {
        std::io::Cursor::new(self.data.clone())
    }
}

impl<Address: AddressTrait> TestBlob<Address> {
    pub fn new(data: Vec<u8>, address: Address) -> Self {
        Self { address, data }
    }
}
