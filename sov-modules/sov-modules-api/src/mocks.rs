use crate::{Address, Context, PublicKey, SigVerificationError, Signature, Spec};
use borsh::{BorshDeserialize, BorshSerialize};
use jmt::SimpleHasher;
use sov_state::{JmtStorage, ZkStorage};
use std::convert::Infallible;

/// Mock for Spec::PublicKey, useful for testing.
#[derive(PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize, Debug)]
pub struct MockPublicKey {
    pub_key: Vec<u8>,
}

impl MockPublicKey {
    pub fn new(pub_key: Vec<u8>) -> Self {
        Self { pub_key }
    }

    pub fn sign(&self, _msg: [u8; 32]) -> MockSignature {
        MockSignature { msg_sig: vec![] }
    }
}

impl TryFrom<&'static str> for MockPublicKey {
    type Error = Infallible;

    fn try_from(key: &'static str) -> Result<Self, Self::Error> {
        let key = key.as_bytes().to_vec();
        Ok(Self { pub_key: key })
    }
}

impl PublicKey for MockPublicKey {
    fn to_address(&self) -> Address {
        let pub_key_hash = <MockContext as Spec>::Hasher::hash(&self.pub_key);
        Address::new(pub_key_hash)
    }
}

/// Mock for Spec::Signature, useful for testing.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, PartialEq, Eq, Debug, Clone)]
pub struct MockSignature {
    pub msg_sig: Vec<u8>,
}

impl Signature for MockSignature {
    type PublicKey = MockPublicKey;

    fn verify(
        &self,
        _pub_key: &Self::PublicKey,
        _msg_hash: [u8; 32],
    ) -> Result<(), SigVerificationError> {
        Ok(())
    }
}

/// Mock for Context, useful for testing.
#[derive(Clone, Debug, PartialEq)]
pub struct MockContext {
    pub sender: MockPublicKey,
}

impl Spec for MockContext {
    type Storage = JmtStorage<Self::Hasher>;
    type Hasher = sha2::Sha256;
    type PublicKey = MockPublicKey;
    type Signature = MockSignature;
}

impl Context for MockContext {
    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }

    fn new(sender: Self::PublicKey) -> Self {
        Self { sender }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZkMockContext {
    pub sender: MockPublicKey,
}

impl Spec for ZkMockContext {
    type Storage = ZkStorage<Self::Hasher>;
    type Hasher = sha2::Sha256;
    type PublicKey = MockPublicKey;
    type Signature = MockSignature;
}

impl Context for ZkMockContext {
    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }

    fn new(sender: Self::PublicKey) -> Self {
        Self { sender }
    }
}
