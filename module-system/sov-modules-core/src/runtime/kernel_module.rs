//! Traits to allow modular development of kernels. These traits are closely related but to the traits
//! for normal modules.

use crate::{Context, ModuleError, WorkingSet};

/// All the methods have a default implementation that can't be invoked (because they take `NonInstantiable` parameter).
/// This allows developers to override only some of the methods in their implementation and safely ignore the others.
pub trait KernelModule {
    /// Execution context.
    type Context: Context;

    /// Configuration for the genesis method.
    type Config;

    /// Genesis is called when a rollup is deployed and can be used to set initial state values in the module.
    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<Self::Context>,
    ) -> Result<(), ModuleError> {
        Ok(())
    }
}
