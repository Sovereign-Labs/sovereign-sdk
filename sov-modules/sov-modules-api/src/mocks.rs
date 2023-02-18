use std::convert::Infallible;

use crate::Context;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_state::{JmtStorage, ZkStorage};

/// Mock for Context::PublicKey, useful for testing.
#[derive(PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize, Debug)]
pub struct MockPublicKey {
    pub_key: Vec<u8>,
}

impl MockPublicKey {
    pub fn new(pub_key: Vec<u8>) -> Self {
        Self { pub_key }
    }
}

impl TryFrom<&'static str> for MockPublicKey {
    type Error = Infallible;

    fn try_from(key: &'static str) -> Result<Self, Self::Error> {
        let key = key.as_bytes().to_vec();
        Ok(Self { pub_key: key })
    }
}

/// Mock for Context::Signature, useful for testing.
#[derive(borsh::BorshDeserialize, PartialEq, Eq)]
pub struct MockSignature {
    sig: Vec<u8>,
}

impl MockSignature {
    pub fn new(sig: Vec<u8>) -> Self {
        Self { sig }
    }
}

/// Mock for Context, useful for testing.
#[derive(Clone)]
pub struct MockContext {
    pub sender: MockPublicKey,
}

impl Context for MockContext {
    type Storage = JmtStorage;

    type PublicKey = MockPublicKey;

    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }

    fn new(sender: Self::PublicKey) -> Self {
        Self { sender }
    }
}

#[derive(Clone)]
pub struct ZkMockContext {
    pub sender: MockPublicKey,
}

impl Context for ZkMockContext {
    type Storage = ZkStorage;

    type PublicKey = MockPublicKey;

    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }

    fn new(sender: Self::PublicKey) -> Self {
        Self { sender }
    }
}
