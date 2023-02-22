use crate::{Context, Error, Spec};

/// Methods from this trait should be called only once during the rollup deployment.
pub trait Genesis {
    type Context: Context;

    /// Initializes the state of the rollup.
    // TDOD: genesis should take initial configuration as an argument.
    fn genesis() -> Result<<<Self as Genesis>::Context as Spec>::Storage, Error>;
}
