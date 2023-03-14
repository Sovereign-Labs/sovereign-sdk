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
pub use prefix::Prefix;
pub use response::{CallResponse, QueryResponse};

use sov_state::Storage;
use sovereign_sdk::serial::{Decode, Encode};
use std::fmt::Debug;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SigVerificationError {
    #[error("Bad signature")]
    BadSignature,
}

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

/// Spec contains types common for all modules.
pub trait Spec {
    type Storage: Storage + Clone;

    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + TryFrom<&'static str>
        + Clone
        + Debug;

    type Hasher: jmt::SimpleHasher;

    type Signature: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + Clone
        + Debug
        + Signature<PublicKey = Self::PublicKey>;
}

/// Context contains functionality common for all modules.
pub trait Context: Spec + Clone + Debug + PartialEq {
    /// Sender of the transaction.
    fn sender(&self) -> &Self::PublicKey;

    /// Constructor for the Context.
    fn new(sender: Self::PublicKey) -> Self;
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
    fn genesis(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// Call allows interaction with the module and invokes state changes.
    /// It takes a module defined type and a context as parameters.
    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: &Self::Context,
    ) -> Result<CallResponse, Error> {
        unreachable!()
    }

    /// Query allows querying the module's state.
    fn query(&self, _message: Self::QueryMessage) -> QueryResponse {
        unreachable!()
    }
}

/// Every module has to implement this trait.
/// It defines the `new` method for now and can be extended with some other metadata in the future.
pub trait ModuleInfo<C: Context> {
    fn new(storage: C::Storage) -> Self;
}
