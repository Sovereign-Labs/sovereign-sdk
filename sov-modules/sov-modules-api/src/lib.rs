#![feature(associated_type_defaults)]

pub mod default_context;
pub mod default_signature;

mod bech32;
mod dispatch;
mod encode;
mod error;
mod prefix;
mod response;

#[cfg(test)]
mod tests;
pub use crate::bech32::AddressBech32;
pub use dispatch::{DispatchCall, DispatchQuery, Genesis};
pub use error::Error;
pub use jmt::SimpleHasher as Hasher;

pub use prefix::Prefix;
pub use response::{CallResponse, QueryResponse};

use sov_state::{Storage, Witness, WorkingSet};
pub use sovereign_sdk::core::traits::AddressTrait;

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt::{self, Debug, Display};
use thiserror::Error;

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl AddressTrait for Address {}

#[derive(
    Debug,
    PartialEq,
    Clone,
    Eq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]
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

#[derive(Error, Debug)]
pub enum SigVerificationError {
    #[error("Bad signature {0}")]
    BadSignature(String),
}

/// Signature used in the module system.
pub trait Signature {
    type PublicKey;

    fn verify(
        &self,
        pub_key: &Self::PublicKey,
        msg_hash: [u8; 32],
    ) -> Result<(), SigVerificationError>;
}

/// A type that can't be instantiated.
#[derive(Debug, PartialEq)]
pub enum NonInstantiable {}

/// PublicKey used in the module system.
pub trait PublicKey {
    fn to_address<A: AddressTrait>(&self) -> A;
}

/// Spec contains types common for all modules.
pub trait Spec {
    #[cfg(feature = "native")]
    type Address: AddressTrait
        + BorshSerialize
        + BorshDeserialize
        + Into<AddressBech32>
        + for<'a> serde::Deserialize<'a>;

    #[cfg(not(feature = "native"))]
    type Address: AddressTrait + BorshSerialize + BorshDeserialize;

    type Storage: Storage + Clone;

    type PublicKey: borsh::BorshDeserialize + borsh::BorshSerialize + Eq + Clone + Debug + PublicKey;

    type Hasher: Hasher;

    type Signature: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + Signature<PublicKey = Self::PublicKey>;

    type Witness: Witness;
}

/// Context contains functionality common for all modules.
pub trait Context: Spec + Clone + Debug + PartialEq {
    /// Sender of the transaction.
    fn sender(&self) -> &Self::Address;

    /// Constructor for the Context.
    fn new(sender: Self::Address) -> Self;
}

/// Every module has to implement this trait.
/// All the methods have a default implementation that can't be invoked (because they take `NonInstantiable` parameter).
/// This allows developers to override only some of the methods in their implementation and safely ignore the others.
pub trait Module {
    /// Types and functionality common for all modules:
    type Context: Context;

    /// Types and functionality defined per module:

    /// Configuration for the genesis method.
    type Config;

    /// Module defined argument to the call method.
    type CallMessage: Debug + BorshSerialize + BorshDeserialize = NonInstantiable;

    /// Module defined argument to the query method.
    type QueryMessage: Debug + BorshSerialize + BorshDeserialize = NonInstantiable;

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

    /// Query allows querying the module's state.
    fn query(
        &self,
        _message: Self::QueryMessage,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> QueryResponse {
        unreachable!()
    }
}

/// Every module has to implement this trait.
pub trait ModuleInfo {
    type Context: Context;

    fn new() -> Self;

    fn address(&self) -> &<Self::Context as Spec>::Address;
}
