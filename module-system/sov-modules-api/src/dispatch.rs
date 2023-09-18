use crate::{CallResponse, Context, Error, Module, Spec, WorkingSet};

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
    ) -> Result<(), Error>;
}

/// A trait that needs to be implemented for any call message.
pub trait DispatchCall {
    type Context: Context;
    type Decodable;

    /// Decodes serialized call message
    fn decode_call(serialized_message: &[u8]) -> Result<Self::Decodable, std::io::Error>;

    /// Dispatches a call message to the appropriate module.
    fn dispatch_call(
        &self,
        message: Self::Decodable,
        working_set: &mut WorkingSet<Self::Context>,
        context: &Self::Context,
    ) -> Result<CallResponse, Error>;

    /// Returns an address of the dispatched module.
    fn module_address(&self, message: &Self::Decodable) -> &<Self::Context as Spec>::Address;
}

/// A trait that specifies how a runtime should encode the data for each module
pub trait EncodeCall<M: Module> {
    /// The encoding function
    fn encode_call(data: M::CallMessage) -> Vec<u8>;
}

/// A trait that needs to be implemented for a *runtime* to be used with the CLI wallet
#[cfg(feature = "native")]
pub trait CliWallet: DispatchCall {
    /// The type that is used to represent this type in the CLI. Typically,
    /// this type implements the clap::Subcommand trait. This type is generic to
    /// allow for different representations of the same type in the interface; a
    /// typical end-usage will impl traits only in the case where `CliStringRepr<T>: Into::RuntimeCall`
    type CliStringRepr<T>;
}
