use borsh::{BorshDeserialize, BorshSerialize};

use crate::zk::traits::{Matches, ProofTrait, Zkvm};

use super::{traits::Witness, types::ArrayWitness};

#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize)]
pub struct MockCodeCommitment(pub [u8; 32]);

impl Matches<MockCodeCommitment> for MockCodeCommitment {
    fn matches(&self, other: &MockCodeCommitment) -> bool {
        self.0 == other.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize, BorshSerialize)]
pub struct MockProof {
    program_id: MockCodeCommitment,
    log: Vec<u8>,
}

impl ProofTrait<MockZkvm> for MockProof {
    type Output = Vec<u8>;

    fn verify(self, code_commitment: &MockCodeCommitment) -> Result<Self::Output, anyhow::Error> {
        if !self.program_id.matches(code_commitment) {
            anyhow::bail!("Invalid code commitment")
        }
        Ok(self.log)
    }
}

pub struct MockZkvm(ArrayWitness);

impl Zkvm for MockZkvm {
    type CodeCommitment = MockCodeCommitment;

    type Proof = MockProof;

    type Error = anyhow::Error;

    fn write_to_guest<T: crate::serial::Encode>(&self, hint: T) {
        self.0.add_hint(hint)
    }

    fn read_from_host<T: crate::serial::Decode>(&self) -> T {
        self.0.get_hint()
    }

    fn verify(
        proof: Self::Proof,
        code_commitment: &Self::CodeCommitment,
    ) -> Result<<<Self as Zkvm>::Proof as crate::zk::traits::ProofTrait<Self>>::Output, Self::Error>
    {
        proof.verify(code_commitment)
    }
}
