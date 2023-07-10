#![feature(associated_type_defaults)]

mod bech32;
pub mod default_context;
pub mod default_signature;
mod dispatch;
mod encode;
mod error;
pub mod hooks;
mod prefix;
mod response;
mod serde_address;
#[cfg(test)]
mod tests;
pub mod transaction;

use core::fmt::{self, Debug, Display};

use borsh::{BorshDeserialize, BorshSerialize};
pub use dispatch::{DispatchCall, Genesis};
pub use error::Error;
pub use prefix::Prefix;
pub use response::CallResponse;
use serde::{Deserialize, Serialize};
pub use sov_rollup_interface::crypto::SimpleHasher as Hasher;
pub use sov_rollup_interface::traits::AddressTrait;
use sov_state::{Storage, Witness, WorkingSet};
use thiserror::Error;

pub use crate::bech32::AddressBech32;

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl AddressTrait for Address {}

#[derive(PartialEq, Clone, Eq, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Address {
    addr: [u8; 32],
}

impl<'a> TryFrom<&'a [u8]> for Address {
    type Error = anyhow::Error;

    fn try_from(addr: &'a [u8]) -> Result<Self, Self::Error> {
        if addr.len() != 32 {
            anyhow::bail!("Address must be 32 bytes long");
        }
        let mut addr_bytes = [0u8; 32];
        addr_bytes.copy_from_slice(addr);
        Ok(Self { addr: addr_bytes })
    }
}

impl From<[u8; 32]> for Address {
    fn from(addr: [u8; 32]) -> Self {
        Self { addr }
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", AddressBech32::from(self))
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", AddressBech32::from(self))
    }
}

impl From<AddressBech32> for Address {
    fn from(addr: AddressBech32) -> Self {
        Self {
            addr: addr.to_byte_array(),
        }
    }
}

#[derive(Error, Debug)]
pub enum SigVerificationError {
    #[error("Bad signature {0}")]
    BadSignature(String),
}

/// Signature used in the Module System.
pub trait Signature {
    type PublicKey;

    fn verify(
        &self,
        pub_key: &Self::PublicKey,
        msg_hash: [u8; 32],
    ) -> Result<(), SigVerificationError>;
}

/// A type that can't be instantiated.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum NonInstantiable {}

/// PublicKey used in the Module System.
pub trait PublicKey {
    fn to_address<A: AddressTrait>(&self) -> A;
}

/// The `Spec` trait configures certain key primitives to be used by a by a particular instance of a rollup.
/// `Spec` is almost always implemented on a Context object; since all Modules are generic
/// over a Context, rollup developers can easily optimize their code for different environments
/// by simply swapping out the Context (and by extension, the Spec).
///
/// For example, a rollup running in a STARK-based zkvm like Risc0 might pick Sha256 or Poseidon as its preferred hasher,
/// while a rollup running in an elliptic-curve based SNARK such as `Placeholder` from the =nil; foundation might
/// prefer a Pedersen hash. By using a generic Context and Spec, a rollup developer can trivially customize their
/// code for either (or both!) of these environments without touching their module implementations.
pub trait Spec {
    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(feature = "native")]
    type Address: AddressTrait
        + BorshSerialize
        + BorshDeserialize
        + Into<AddressBech32>
        + From<AddressBech32>;

    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(not(feature = "native"))]
    type Address: AddressTrait + BorshSerialize + BorshDeserialize;

    /// Authenticated state storage used by the rollup. Typically some variant of a merkle-patricia trie.
    type Storage: Storage + Clone + Send + Sync;

    /// The public key used for digital signatures
    #[cfg(feature = "native")]
    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + PublicKey
        + Serialize
        + for<'a> Deserialize<'a>
        + Send
        + Sync;

    #[cfg(not(feature = "native"))]
    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + Send
        + Sync
        + PublicKey;

    /// The hasher preferred by the rollup, such as Sha256 or Poseidon.
    type Hasher: Hasher;

    /// The digital signature scheme used by the rollup
    type Signature: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + Signature<PublicKey = Self::PublicKey>;

    /// A structure containing the non-deterministic inputs from the prover to the zk-circuit
    type Witness: Witness;
}

/// A context contains information which is passed to modules during
/// transaction execution. Currently, context includes the sender of the transaction
/// as recovered from its signature.
///
/// Context objects also implement the [`Spec`] trait, which specifies the types to be used in this
/// instance of the state transition function. By making modules generic over a `Context`, developers
/// can easily update their cryptography to conform to the needs of different zk-proof systems.
pub trait Context: Spec + Clone + Debug + PartialEq {
    /// Sender of the transaction.
    fn sender(&self) -> &Self::Address;

    /// Constructor for the Context.
    fn new(sender: Self::Address) -> Self;
}

impl<T> Genesis for T
where
    T: Module,
{
    type Context = <Self as Module>::Context;

    type Config = <Self as Module>::Config;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<<<Self as Genesis>::Context as Spec>::Storage>,
    ) -> Result<(), Error> {
        <Self as Module>::genesis(self, config, working_set)
    }
}

/// All the methods have a default implementation that can't be invoked (because they take `NonInstantiable` parameter).
/// This allows developers to override only some of the methods in their implementation and safely ignore the others.
pub trait Module {
    /// Execution context.
    type Context: Context;

    /// Configuration for the genesis method.
    type Config;

    /// Module defined argument to the call method.
    type CallMessage: Debug + BorshSerialize + BorshDeserialize = NonInstantiable;

    /// Genesis is called when a rollup is deployed and can be used to set initial state values in the module.
    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Call allows interaction with the module and invokes state changes.
    /// It takes a module defined type and a context as parameters.
    fn call(
        &self,
        _message: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> Result<CallResponse, Error> {
        unreachable!()
    }
}

/// Every module has to implement this trait.
pub trait ModuleInfo: Default {
    type Context: Context;

    /// Returns address of the module.
    fn address(&self) -> &<Self::Context as Spec>::Address;
}

/// A StateTransitionRunner needs to implement this if
/// the RPC service is needed
pub trait RpcRunner {
    type Context: Context;
    fn get_storage(&self) -> <Self::Context as Spec>::Storage;
}
