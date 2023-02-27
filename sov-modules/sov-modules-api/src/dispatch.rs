use crate::{CallResponse, Context, Error, QueryResponse, Spec};

/// Methods from this trait should be called only once during the rollup deployment.
pub trait Genesis {
    type Context: Context;
    type Config;

    /// Initializes the state of the rollup.
    // TDOD: genesis should take initial configuration as an argument.
    fn genesis(
        config: Self::Config,
    ) -> Result<<<Self as Genesis>::Context as Spec>::Storage, Error>;
}

/// A trait that needs to be implemented for any call message.
pub trait DispatchCall {
    type Context: Context;

    /// Dispatches a call message to the appropriate module.
    fn dispatch(
        self,
        storage: <<Self as DispatchCall>::Context as Spec>::Storage,
        context: &Self::Context,
    ) -> Result<CallResponse, Error>;
}

/// A trait that needs to be implemented for any query message.
pub trait DispatchQuery {
    type Context: Context;

    /// Dispatches a query message to the appropriate module.
    fn dispatch(
        self,
        storage: <<Self as DispatchQuery>::Context as Spec>::Storage,
    ) -> QueryResponse;
}
