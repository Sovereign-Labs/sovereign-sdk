#![feature(associated_type_defaults)]
#[cfg(feature = "mocks")]
pub mod mocks;

mod encode;
mod error;
mod prefix;
mod response;

pub use error::Error;
pub use prefix::Prefix;
pub use response::{CallResponse, QueryResponse};

use sov_state::Storage;
use sovereign_sdk::serial::Decode;
use std::fmt::Debug;
/// A type that can't be instantiated.
pub enum NonInstantiable {}

/// Context contains types and functionality common for all modules.
pub trait Context: Clone {
    type Storage: Storage + Clone;

    type PublicKey: borsh::BorshDeserialize
        + borsh::BorshSerialize
        + Eq
        + TryFrom<&'static str>
        + Clone
        + Debug;

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
    type CallMessage: Decode = NonInstantiable;

    /// Module defined argument to the query method.
    type QueryMessage: Decode = NonInstantiable;

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
