//! Runtime module definitions.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::common::{ModuleError, ModulePrefix};
use crate::storage::WorkingSet;

mod dispatch;
mod spec;

pub use dispatch::*;
pub use spec::*;

/// Response type for the `Module::call` method.
#[derive(Default, Debug)]
pub struct CallResponse {}

/// The core trait implemented by all modules. This trait defines how a module is initialized at genesis,
/// and how it handles user transactions (if applicable).
pub trait Module {
    /// Execution context.
    type Context: Context;

    /// Configuration for the genesis method.
    type Config;

    /// Module defined argument to the call method.
    type CallMessage: Debug + BorshSerialize + BorshDeserialize;

    /// Module defined event resulting from a call method.
    type Event: Debug + BorshSerialize + BorshDeserialize;

    /// Genesis is called when a rollup is deployed and can be used to set initial state values in the module.
    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<Self::Context>,
    ) -> Result<(), ModuleError> {
        Ok(())
    }

    /// Call allows interaction with the module and invokes state changes.
    /// It takes a module defined type and a context as parameters.
    fn call(
        &self,
        _message: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<Self::Context>,
    ) -> Result<CallResponse, ModuleError>;

    /// Attempts to charge the provided amount of gas from the working set.
    ///
    /// The scalar gas value will be computed from the price defined on the working set.
    fn charge_gas(
        &self,
        working_set: &mut WorkingSet<Self::Context>,
        gas: &<Self::Context as Context>::GasUnit,
    ) -> anyhow::Result<()> {
        working_set.charge_gas(gas)
    }
}

/// A [`Module`] that has a well-defined and known [JSON
/// Schema](https://json-schema.org/) for its [`Module::CallMessage`].
///
/// This trait is intended to support code generation tools, CLIs, and
/// documentation. You can derive it with `#[derive(ModuleCallJsonSchema)]`, or
/// implement it manually if your use case demands more control over the JSON
/// Schema generation.
pub trait ModuleCallJsonSchema: Module {
    /// Returns the JSON schema for [`Module::CallMessage`].
    fn json_schema() -> String;
}

/// Every module has to implement this trait.
pub trait ModuleInfo {
    /// Execution context.
    type Context: Context;

    /// Returns address of the module.
    fn address(&self) -> &<Self::Context as Spec>::Address;

    /// Returns the prefix of the module.
    fn prefix(&self) -> ModulePrefix;

    /// Returns addresses of all the other modules this module is dependent on
    fn dependencies(&self) -> Vec<&<Self::Context as Spec>::Address>;
}

/// A trait that specifies how a runtime should encode the data for each module
pub trait EncodeCall<M: Module> {
    /// The encoding function
    fn encode_call(data: M::CallMessage) -> Vec<u8>;
}

/// Methods from this trait should be called only once during the rollup deployment.
pub trait Genesis {
    /// Execution context of the module.
    type Context: Context;

    /// Initial configuration for the module.
    type Config;

    /// Initializes the state of the rollup.
    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<Self::Context>,
    ) -> Result<(), ModuleError>;
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
        working_set: &mut WorkingSet<Self::Context>,
    ) -> Result<(), ModuleError> {
        <Self as Module>::genesis(self, config, working_set)
    }
}
