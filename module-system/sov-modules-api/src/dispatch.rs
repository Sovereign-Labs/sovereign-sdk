use sov_state::WorkingSet;

use crate::{CallResponse, Context, Error, Spec};

/// Methods from this trait should be called only once during the rollup deployment.
pub trait Genesis {
    type Context: Context;

    /// Initial configuration for the module.
    type Config;

    /// Initializes the state of the rollup.
    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<<<Self as Genesis>::Context as Spec>::Storage>,
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
        working_set: &mut WorkingSet<<<Self as DispatchCall>::Context as Spec>::Storage>,
        context: &Self::Context,
    ) -> Result<CallResponse, Error>;

    /// Returns an address of the dispatched module.
    fn module_address(&self, message: &Self::Decodable) -> &<Self::Context as Spec>::Address;
}
