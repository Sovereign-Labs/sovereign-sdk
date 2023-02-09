use crate::Context;
use sov_state::JmtStorage;

/// Mock for Context::PublicKey, useful for testing.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, PartialEq, Eq)]
pub struct MockPublicKey {
    pub_key: Vec<u8>,
}

impl MockPublicKey {
    pub fn new(pub_key: Vec<u8>) -> Self {
        Self { pub_key }
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
pub struct MockContext {
    sender: MockPublicKey,
}

impl Context for MockContext {
    type Storage = JmtStorage;

    type Signature = MockSignature;

    type PublicKey = MockPublicKey;

    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }
}
