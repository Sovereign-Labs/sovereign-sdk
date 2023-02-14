#![feature(associated_type_defaults)]

mod encode;
mod error;
#[cfg(feature = "mocks")]
pub mod mocks;
mod prefix;

pub use error::{DecodingError, DispatchError, ModuleError};
pub use prefix::Prefix;
use sov_state::Storage;
use sovereign_sdk::{
    serial::{Decode, Encode},
    stf::{Event, EventKey, EventValue},
};
use std::{fmt::Debug, rc::Rc};

/// Context contains types and functionality common for all modules.
pub trait Context {
    type Storage: Storage + Clone;

    type PublicKey: Decode + Encode + Eq + TryFrom<&'static str>;

    /// Sender of the transaction.
    fn sender(&self) -> &Self::PublicKey;
}

/// A type that can't be instantiated.
pub enum NonInstantiable {}

/// Response type for the `Module::call` method.
#[derive(Default)]
pub struct CallResponse {
    /// Lists of events emitted by a call to a module.
    events: Vec<Event>,
}

impl CallResponse {
    pub fn add_event(&mut self, key: &str, value: &str) {
        let event = Event {
            key: EventKey(Rc::new(key.as_bytes().to_vec())),
            value: EventValue(Rc::new(value.as_bytes().to_vec())),
        };

        self.events.push(event)
    }
}

/// Response type for the `Module::query` method. The response is returned by the relevant RPC call.
#[derive(Default, Debug)]
pub struct QueryResponse {
    pub response: Vec<u8>,
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

    fn genesis(&mut self) -> Result<(), DispatchError> {
        Ok(())
    }

    /// Call allows interaction with the module and invokes state changes.
    /// It takes a module defined type and a context as parameters.
    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: Self::Context,
    ) -> Result<CallResponse, DispatchError> {
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
