#[cfg(feature = "native")]
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sov_rollup_interface::RollupAddress;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::{ArrayWitness, DefaultStorageSpec, ZkStorage};

#[cfg(feature = "native")]
use crate::default_signature::private_key::DefaultPrivateKey;
use crate::default_signature::{DefaultPublicKey, DefaultSignature};
use crate::{Address, Context, PublicKey, Spec, TupleGasUnit};

#[cfg(feature = "native")]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DefaultContext {
    pub sender: Address,
}

#[cfg(feature = "native")]
impl Spec for DefaultContext {
    type Address = Address;
    type Storage = ProverStorage<DefaultStorageSpec>;
    type PrivateKey = DefaultPrivateKey;
    type PublicKey = DefaultPublicKey;
    type Hasher = sha2::Sha256;
    type Signature = DefaultSignature;
    type Witness = ArrayWitness;
}

#[cfg(feature = "native")]
impl Context for DefaultContext {
    type GasUnit = TupleGasUnit<2>;

    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn new(sender: Self::Address) -> Self {
        Self { sender }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "native", derive(Serialize, Deserialize))]
pub struct ZkDefaultContext {
    pub sender: Address,
}

impl Spec for ZkDefaultContext {
    type Address = Address;
    type Storage = ZkStorage<DefaultStorageSpec>;
    type PublicKey = DefaultPublicKey;
    #[cfg(feature = "native")]
    type PrivateKey = DefaultPrivateKey;
    type Hasher = sha2::Sha256;
    type Signature = DefaultSignature;
    type Witness = ArrayWitness;
}

impl Context for ZkDefaultContext {
    type GasUnit = TupleGasUnit<2>;

    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn new(sender: Self::Address) -> Self {
        Self { sender }
    }
}

impl PublicKey for DefaultPublicKey {
    fn to_address<A: RollupAddress>(&self) -> A {
        let pub_key_hash = {
            let mut hasher = <ZkDefaultContext as Spec>::Hasher::new();
            hasher.update(self.pub_key);
            hasher.finalize().into()
        };
        A::from(pub_key_hash)
    }
}
