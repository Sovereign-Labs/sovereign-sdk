#![feature(associated_type_defaults)]
#[cfg(feature = "mocks")]
pub mod mocks;

mod dispatch;
mod encode;
mod error;
mod prefix;
mod response;

pub use dispatch::{DispatchCall, DispatchQuery, Genesis};
pub use error::Error;
pub use jmt::SimpleHasher as Hasher;

pub use prefix::Prefix;
pub use response::{CallResponse, QueryResponse};

use sov_state::{Storage, WorkingSet};
use sovereign_sdk::{
    core::traits::Witness,
    serial::{Decode, Encode},
};

pub use sovereign_sdk::core::traits::AddressTrait;

use std::fmt::Debug;

use thiserror::Error;

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl<'a> TryFrom<&'a [u8]> for Address {
    type Error = anyhow::Error;

    fn try_from(addr: &'a [u8]) -> Result<Self, Self::Error> {
        Ok(Self {
            addr: addr.to_vec(),
        })
    }
}

impl AddressTrait for Address {}

/// Default implementation of AddressTrait for the module system
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone, Eq)]
pub struct Address {
    addr: Vec<u8>,
}

#[derive(Error, Debug)]
pub enum SigVerificationError {
    #[error("Bad signature")]
    BadSignature,
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
#[derive(Debug)]
pub enum NonInstantiable {}

/// PublicKey used in the module system.
pub trait PublicKey {
    fn to_address<A: AddressTrait>(&self) -> A;
}

/// Spec contains types common for all modules.
pub trait Spec {
    type Address: AddressTrait + borsh::BorshDeserialize + borsh::BorshSerialize;

    type Storage: Storage + Clone;

    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + TryFrom<&'static str>
        + Clone
        + Debug
        + PublicKey;

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

    /// Module defined argument to the call method.
    type CallMessage: Decode + Encode + Debug = NonInstantiable;

    /// Module defined argument to the query method.
    type QueryMessage: Decode + Encode + Debug = NonInstantiable;

    /// Genesis is called when a rollup is deployed and can be used to set initial state values in the module.
    fn genesis(
        &self,
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

    // Returns an address for the module.
    // TODO: https://github.com/Sovereign-Labs/sovereign/issues/136
    fn address(&self) -> &<Self::Context as Spec>::Address;
}
