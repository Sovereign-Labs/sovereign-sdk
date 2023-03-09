use std::convert::Infallible;

use crate::{Context, Spec};
use borsh::{BorshDeserialize, BorshSerialize};
use jmt::SimpleHasher;
use sov_state::{JmtStorage, ZkStorage};
use sovereign_sdk::core::traits::{CanonicalHash, TransactionTrait};

/// Mock for Spec::PublicKey, useful for testing.
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

#[derive(Debug, PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize)]
pub struct Transaction {
    pub mock_signature: MockSignature,
    pub msg: Vec<u8>,
}

impl Transaction {
    pub fn new(msg: Vec<u8>, pub_key: MockPublicKey) -> Self {
        let hash = <MockContext as Spec>::Hasher::hash(&msg);

        Self {
            mock_signature: MockSignature {
                pub_key,
                msg_hash: hash,
            },
            msg,
        }
    }

    pub fn msg(&self) -> &[u8] {
        &self.msg
    }

    pub fn sender(&self) -> &MockPublicKey {
        &self.mock_signature.pub_key
    }
}

impl TransactionTrait for Transaction {
    type Hash = [u8; 32];
}

impl CanonicalHash for Transaction {
    type Output = [u8; 32];

    fn hash(&self) -> Self::Output {
        self.mock_signature.msg_hash
    }
}

/// Mock for Spec::Signature, useful for testing.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, PartialEq, Eq, Debug, Clone)]
pub struct MockSignature {
    pub pub_key: MockPublicKey,
    pub msg_hash: [u8; 32],
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
