use sov_modules_core::{Context, Module, ModuleError, WorkingSet};

/// Methods from this trait should be called only once during the rollup deployment.
pub trait Genesis {
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

/// A trait that specifies how a runtime should encode the data for each module
pub trait EncodeCall<M: Module> {
    /// The encoding function
    fn encode_call(data: M::CallMessage) -> Vec<u8>;
}

/// A trait that needs to be implemented for a *runtime* to be used with the CLI wallet
#[cfg(feature = "native")]
pub trait CliWallet: sov_modules_core::DispatchCall {
    /// The type that is used to represent this type in the CLI. Typically,
    /// this type implements the clap::Subcommand trait. This type is generic to
    /// allow for different representations of the same type in the interface; a
    /// typical end-usage will impl traits only in the case where `CliStringRepr<T>: Into::RuntimeCall`
    type CliStringRepr<T>;
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
