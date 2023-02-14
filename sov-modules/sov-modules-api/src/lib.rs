#![feature(associated_type_defaults)]

#[cfg(feature = "mocks")]
pub mod mocks;

use sov_state::Storage;
use sovereign_sdk::{
    serial::{Decode, DecodeBorrowed, Encode},
    stf::Event,
};
use std::{
    convert::Infallible,
    fmt::{Debug, Display},
    io::Read,
};

// separator == "/"
const DOMAIN_SEPARATOR: [u8; 1] = [47];

/// A unique identifier for each state variable in a module.
#[derive(Debug, PartialEq, Eq)]
pub struct Prefix {
    module_path: &'static str,
    module_name: &'static str,
    storage_name: &'static str,
}

impl Prefix {
    pub fn new(
        module_path: &'static str,
        module_name: &'static str,
        storage_name: &'static str,
    ) -> Self {
        Self {
            module_path,
            module_name,
            storage_name,
        }
    }
}

impl From<Prefix> for sov_state::Prefix {
    fn from(prefix: Prefix) -> Self {
        let mut combined_prefix = Vec::with_capacity(
            prefix.module_path.len()
                + prefix.module_name.len()
                + prefix.storage_name.len()
                + 3 * DOMAIN_SEPARATOR.len(),
        );

        // We call this logic only once per module instantiation, so we don't have to use AlignedVec here.
        combined_prefix.extend(prefix.module_path.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix.extend(prefix.module_name.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        combined_prefix.extend(prefix.storage_name.as_bytes());
        combined_prefix.extend(DOMAIN_SEPARATOR);
        sov_state::Prefix::new(combined_prefix)
    }
}

/// Any kind of error during value decoding.
#[derive(Debug)]
pub struct DecodingError {}

pub enum DispatchError {
    Module(ModuleError),
}

impl Debug for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Module(e) => f.debug_tuple("Module").field(&e.err).finish(),
        }
    }
}

pub struct ModuleError {
    pub err: String,
}

impl From<ModuleError> for DispatchError {
    fn from(err: ModuleError) -> Self {
        Self::Module(err)
    }
}

impl<T: Debug> From<T> for ModuleError {
    fn from(t: T) -> Self {
        Self {
            err: format!("{t:?}"),
        }
    }
}

impl From<Infallible> for DecodingError {
    fn from(_value: Infallible) -> Self {
        unreachable!()
    }
}

/// Context contains types and functionality common for all modules.
pub trait Context {
    type Storage: Storage + Clone;

    type PublicKey: Decode + Encode + Eq + TryFrom<&'static str>;

    /// Sender of the transaction.
    fn sender(&self) -> &Self::PublicKey;
}

/// A type that can't be instantiated.
pub enum NonInstantiable {}

impl<'de> DecodeBorrowed<'de> for NonInstantiable {
    type Error = DecodingError;

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        unreachable!()
    }
}

impl Decode for NonInstantiable {
    type Error = DecodingError;

    fn decode<R: Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        unreachable!()
    }
}
/// Response type for the `Module::call` method.
#[derive(Default)]
pub struct CallResponse {
    /// Lists of events emitted by a call to a module.
    events: Vec<Event>,
}

impl CallResponse {
    pub fn add_event(&mut self, _key: &str, _value: &str) {
        //self.events.push(event)
        //todo!()
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
