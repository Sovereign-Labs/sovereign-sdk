use std::fmt::Debug;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sov_modules_core::{Address, Context, PublicKey, Spec, TupleGasUnit};
use sov_rollup_interface::RollupAddress;
#[cfg(feature = "native")]
use sov_state::ProverStorage;
use sov_state::{ArrayWitness, DefaultStorageSpec, ZkStorage};

#[cfg(feature = "native")]
use crate::default_signature::private_key::DefaultPrivateKey;
use crate::default_signature::{DefaultPublicKey, DefaultSignature};

#[cfg(feature = "native")]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DefaultContext {
    pub sender: Address,
    pub sequencer: Address,
    /// The height to report. This is set by the kernel when the context is created
    visible_height: u64,
}

#[cfg(feature = "native")]
impl Spec for DefaultContext {
    type Address = Address;
    type Storage = ProverStorage<DefaultStorageSpec, sov_prover_storage_manager::SnapshotManager>;
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

    fn sequencer(&self) -> &Self::Address {
        &self.sequencer
    }

    fn new(sender: Self::Address, sequencer: Self::Address, height: u64) -> Self {
        Self {
            sender,
            sequencer,
            visible_height: height,
        }
    }

    fn slot_height(&self) -> u64 {
        self.visible_height
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ZkDefaultContext {
    pub sender: Address,
    pub sequencer: Address,
    /// The height to report. This is set by the kernel when the context is created
    visible_height: u64,
}

impl Spec for ZkDefaultContext {
    type Address = Address;
    type Storage = ZkStorage<DefaultStorageSpec>;
    #[cfg(feature = "native")]
    type PrivateKey = DefaultPrivateKey;
    type PublicKey = DefaultPublicKey;
    type Hasher = sha2::Sha256;
    type Signature = DefaultSignature;
    type Witness = ArrayWitness;
}

impl Context for ZkDefaultContext {
    type GasUnit = TupleGasUnit<2>;

    fn sender(&self) -> &Self::Address {
        &self.sender
    }

    fn sequencer(&self) -> &Self::Address {
        &self.sequencer
    }

    fn new(sender: Self::Address, sequencer: Self::Address, height: u64) -> Self {
        Self {
            sender,
            sequencer,
            visible_height: height,
        }
    }

    fn slot_height(&self) -> u64 {
        self.visible_height
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
