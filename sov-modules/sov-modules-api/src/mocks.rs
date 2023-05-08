use crate::{Address, AddressTrait, Context, PublicKey, SigVerificationError, Signature, Spec};
use borsh::{BorshDeserialize, BorshSerialize};
use jmt::SimpleHasher;
#[cfg(feature = "native")]
use serde::{Deserialize, Serialize};
use sov_state::mocks::DefaultStorageSpec;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::ZkStorage;
use sovereign_sdk::core::types::ArrayWitness;

#[derive(PartialEq, Eq, Clone, BorshDeserialize, BorshSerialize, Debug)]
pub struct DefaultPublicKey {
    pub_key: Vec<u8>,
}

impl DefaultPublicKey {
    pub fn new(pub_key: Vec<u8>) -> Self {
        Self { pub_key }
    }

    pub fn sign(&self, _msg: [u8; 32]) -> DefaultSignature {
        DefaultSignature {
            msg_sig: vec![],
            should_fail: false,
        }
    }
}

impl<T: AsRef<str>> From<T> for DefaultPublicKey {
    fn from(key: T) -> Self {
        let key = key.as_ref().as_bytes().to_vec();
        Self { pub_key: key }
    }
}

impl PublicKey for DefaultPublicKey {
    fn to_address<A: AddressTrait>(&self) -> A {
        let pub_key_hash = <ZkDefaultContext as Spec>::Hasher::hash(&self.pub_key);
        A::try_from(&pub_key_hash).expect("todo")
    }
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, PartialEq, Eq, Debug, Clone, Default)]
pub struct DefaultSignature {
    pub msg_sig: Vec<u8>,
    pub should_fail: bool,
}

impl Signature for DefaultSignature {
    type PublicKey = DefaultPublicKey;

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

// TODO: consider feature gating the serde implementations, since they are only needed for RPC
// https://github.com/Sovereign-Labs/sovereign/issues/175
#[cfg(feature = "native")]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DefaultContext {
    pub sender: Address,
}

#[cfg(feature = "native")]
impl Spec for DefaultContext {
    type Address = Address;
    type Storage = ProverStorage<DefaultStorageSpec>;
    type PublicKey = DefaultPublicKey;
    type Hasher = sha2::Sha256;
    type Signature = DefaultSignature;
    type Witness = ArrayWitness;
}

#[cfg(feature = "native")]
impl Context for DefaultContext {
    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn new(sender: Self::Address) -> Self {
        Self { sender }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZkDefaultContext {
    pub sender: Address,
}

impl Spec for ZkDefaultContext {
    type Address = Address;
    type Storage = ZkStorage<DefaultStorageSpec>;
    type PublicKey = DefaultPublicKey;
    type Hasher = sha2::Sha256;
    type Signature = DefaultSignature;
    type Witness = ArrayWitness;
}

impl Context for ZkDefaultContext {
    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn new(sender: Self::Address) -> Self {
        Self { sender }
    }
}
