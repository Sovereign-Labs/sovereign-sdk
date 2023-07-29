#[cfg(feature = "native")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "native")]
use sov_rollup_interface::crypto::SimpleHasher;
#[cfg(feature = "native")]
use sov_rollup_interface::AddressTrait;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
#[cfg(feature = "zk")]
use sov_state::ZkStorage;
#[cfg(feature = "native")]
use sov_state::{ArrayWitness, DefaultStorageSpec};

#[cfg(feature = "native")]
use crate::default_signature::{DefaultPublicKey, DefaultSignature};
#[cfg(feature = "native")]
use crate::{Address, Context, PublicKey, Spec};

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

#[cfg(feature = "zk")]
#[derive(Clone, Debug, PartialEq)]
pub struct ZkDefaultContext {
    pub sender: Address,
}

#[cfg(feature = "zk")]
impl Spec for ZkDefaultContext {
    type Address = Address;
    type Storage = ZkStorage<DefaultStorageSpec>;
    type PublicKey = DefaultPublicKey;
    type Hasher = sha2::Sha256;
    type Signature = DefaultSignature;
    type Witness = ArrayWitness;
}

#[cfg(feature = "zk")]
impl Context for ZkDefaultContext {
    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn new(sender: Self::Address) -> Self {
        Self { sender }
    }
}

#[cfg(feature = "native")]
impl PublicKey for DefaultPublicKey {
    fn to_address<A: AddressTrait>(&self) -> A {
        let pub_key_hash = <DefaultContext as Spec>::Hasher::hash(self.pub_key);
        A::from(pub_key_hash)
    }
}
