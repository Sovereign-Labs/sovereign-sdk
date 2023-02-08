#![feature(associated_type_defaults)]

#[cfg(feature = "mocks")]
pub mod mocks;

use sov_state::Storage;
use sovereign_sdk::{
    serial::{Decode, DecodeBorrowed, Encode},
    stf::Event,
};
use std::{convert::Infallible, io::Read};

// A unique identifier for each state variable in a module.
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
            prefix.module_path.len() + prefix.module_name.len() + prefix.storage_name.len() + 3,
        );

        // separator == "/""
        let separator = [47];

        // We call this logic only once per module instantiation, so we don't have to use AlignedVec here.
        combined_prefix.extend(prefix.module_path.as_bytes());
        combined_prefix.extend(separator);
        combined_prefix.extend(prefix.module_name.as_bytes());
        combined_prefix.extend(separator);
        combined_prefix.extend(prefix.storage_name.as_bytes());
        combined_prefix.extend(separator);
        sov_state::Prefix::new(combined_prefix)
    }
}

// Any kind of error during value decoding.
#[derive(Debug)]
pub struct DecodingError {}

impl From<Infallible> for DecodingError {
    fn from(_value: Infallible) -> Self {
        unreachable!()
    }
}

// Context contains types and functionality common for all modules.
pub trait Context {
    type Storage: Storage + Clone;
    type Signature: Decode;
    type PublicKey: Decode + Encode + Eq;

    // Sender of the transaction.
    fn sender(&self) -> &Self::PublicKey;
}

// A type that can't be instantiated.
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
// Response type for the `Module::call` method.
#[derive(Default)]
pub struct CallResponse {
    // Lists of events emitted by a call to a module.
    pub events: Vec<Event>,
}

// Response type for the `Module::query` method. The response is returned by the relevant RPC call.
#[derive(Default)]
pub struct QueryResponse {}

// Every module has to implement this trait.
// All the methods have a default implementation that can't be invoked (because they take `NonInstantiable` parameter).
// This allows developers to override only some of the methods in their implementation and safely ignore the others.

pub trait Module {
    // Types and functionality common for all modules:
    type Context: Context;

    // Types and functionality defined per module:

    // Module defined argument to the init method.
    type InitMessage: Decode = NonInstantiable;

    // Module defined argument to the call method.
    type CallMessage: Decode = NonInstantiable;

    // Module defined argument to the query method.
    type QueryMessage: Decode = NonInstantiable;

    // Error type for the call method.
    type CallError: Into<DecodingError> = Infallible;

    // Error type for the query method.
    type QueryError: Into<DecodingError> = Infallible;

    // Init is called once per module liftime and can be used to set initial state values in the module.
    // It takes a module defined type and a context as parameters.
    fn init(
        &mut self,
        _message: Self::InitMessage,
        _context: Self::Context,
    ) -> Result<CallResponse, Self::CallError> {
        unreachable!()
    }

    // Call allows interaction with the module and invokes state changes.
    // It takes a module defined type and a context as parameters.
    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: Self::Context,
    ) -> Result<CallResponse, Self::CallError> {
        unreachable!()
    }

    // Query allows querying the module's state.
    fn query(&self, _message: Self::QueryMessage) -> Result<QueryResponse, Self::QueryError> {
        unreachable!()
    }
}
