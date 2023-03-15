use sov_state::WorkingSet;

use crate::{CallResponse, Context, Error, QueryResponse, Spec};

/// Methods from this trait should be called only once during the rollup deployment.
pub trait Genesis {
    type Context: Context;

    /// Initializes the state of the rollup.
    fn genesis(
        working_set: WorkingSet<<<Self as Genesis>::Context as Spec>::Storage>,
    ) -> Result<(), Error>;
}

/// A trait that needs to be implemented for any call message.
pub trait DispatchCall {
    type Context: Context;

    /// Dispatches a call message to the appropriate module.
    fn dispatch_call(
        self,
        working_set: WorkingSet<<<Self as DispatchCall>::Context as Spec>::Storage>,
        context: &Self::Context,
    ) -> Result<CallResponse, Error>;
}

/// A trait that needs to be implemented for any query message.
pub trait DispatchQuery {
    type Context: Context;

    /// Dispatches a query message to the appropriate module.
    fn dispatch_query(
        self,
        working_set: WorkingSet<<<Self as DispatchQuery>::Context as Spec>::Storage>,
    ) -> QueryResponse;
}
