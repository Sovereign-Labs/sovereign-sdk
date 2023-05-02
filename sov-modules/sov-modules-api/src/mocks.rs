use crate::{Address, AddressTrait, Context, PublicKey, SigVerificationError, Signature, Spec};
use borsh::{BorshDeserialize, BorshSerialize};
use jmt::SimpleHasher;
#[cfg(feature = "native")]
use serde::{Deserialize, Serialize};
use sov_state::mocks::MockStorageSpec;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::ZkStorage;
use sovereign_sdk::core::types::ArrayWitness;
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
        MockSignature {
            msg_sig: vec![],
            should_fail: false,
        }
    }
}

impl TryFrom<&str> for MockPublicKey {
    type Error = Infallible;

    fn try_from(key: &str) -> Result<Self, Self::Error> {
        let key = key.as_bytes().to_vec();
        Ok(Self { pub_key: key })
    }
}

impl TryFrom<String> for MockPublicKey {
    type Error = Infallible;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        let key = key.as_bytes().to_vec();
        Ok(Self { pub_key: key })
    }
}

impl PublicKey for MockPublicKey {
    fn to_address<A: AddressTrait>(&self) -> A {
        let mut hasher = sha2::Sha256::new();
        hasher.update(&self.pub_key);
        let hash = hasher.finalize();
        A::try_from(&hash).expect("todo")
    }
}

/// Mock for Spec::Signature, useful for testing.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, PartialEq, Eq, Debug, Clone, Default)]
pub struct MockSignature {
    pub msg_sig: Vec<u8>,
    pub should_fail: bool,
}

impl Signature for MockSignature {
    type PublicKey = MockPublicKey;

    fn verify(
        &self,
        _pub_key: &Self::PublicKey,
        _msg_hash: [u8; 32],
    ) -> Result<(), SigVerificationError> {
        if self.should_fail {
            Err(SigVerificationError::BadSignature)
        } else {
            Ok(())
        }
    }
}

/// Mock for Context, useful for testing.
// TODO: consider feature gating the serde implementations, since they are only needed for RPC
// https://github.com/Sovereign-Labs/sovereign/issues/175
#[cfg(feature = "native")]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MockContext {
    pub sender: Address,
}

#[cfg(feature = "native")]
impl Spec for MockContext {
    type Address = Address;
    type Storage = ProverStorage<MockStorageSpec>;
    type PublicKey = MockPublicKey;
    type Hasher = sha2::Sha256;
    type Signature = MockSignature;
    type Witness = ArrayWitness;
}

#[cfg(feature = "native")]
impl Context for MockContext {
    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn new(sender: Self::Address) -> Self {
        Self { sender }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZkMockContext {
    pub sender: Address,
}

impl Spec for ZkMockContext {
    type Address = Address;
    type Storage = ZkStorage<MockStorageSpec>;
    type PublicKey = MockPublicKey;
    type Hasher = sha2::Sha256;
    type Signature = MockSignature;
    type Witness = ArrayWitness;
}

impl Context for ZkMockContext {
    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn new(sender: Self::Address) -> Self {
        Self { sender }
    }
}
