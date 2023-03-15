use crate::{Address, Context, PublicKey, SigVerificationError, Signature, Spec};
use borsh::{BorshDeserialize, BorshSerialize};
use jmt::SimpleHasher;
use sov_state::ZkStorage;
use std::convert::Infallible;
use std::{cell::RefCell, sync::atomic::AtomicUsize};

use crate::Witness;
use sov_state::{mocks::MockStorageSpec, ProverStorage};
use sovereign_sdk::serial::{Decode, Encode};

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
    type Storage = ProverStorage<MockStorageSpec>;
    type Hasher = sha2::Sha256;
    type PublicKey = MockPublicKey;
    type Signature = MockSignature;
    type Witness = MockWitness;
}

impl Context for MockContext {
    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }

    fn new(sender: Self::PublicKey) -> Self {
        Self { sender }
    }
}

#[derive(Default)]
pub struct MockWitness {
    next_idx: AtomicUsize,
    hints: RefCell<Vec<Vec<u8>>>,
}

impl Witness for MockWitness {
    fn add_hint<T: Encode + Decode>(&self, hint: T) {
        self.hints.borrow_mut().push(hint.encode_to_vec())
    }

    fn get_hint<T: Encode + Decode>(&self) -> T {
        let idx = self
            .next_idx
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        T::decode_from_slice(&self.hints.borrow()[idx]).unwrap()
    }

    fn merge(&self, rhs: &Self) {
        let rhs_next_idx = rhs.next_idx.load(std::sync::atomic::Ordering::SeqCst);
        self.hints
            .borrow_mut()
            .extend(rhs.hints.borrow_mut().drain(rhs_next_idx..))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZkMockContext {
    pub sender: MockPublicKey,
}

impl Spec for ZkMockContext {
    type Storage = ZkStorage<MockStorageSpec>;
    type Hasher = sha2::Sha256;
    type PublicKey = MockPublicKey;
    type Signature = MockSignature;
    type Witness = MockWitness;
}

impl Context for ZkMockContext {
    fn sender(&self) -> &Self::PublicKey {
        &self.sender
    }

    fn new(sender: Self::PublicKey) -> Self {
        Self { sender }
    }
}
