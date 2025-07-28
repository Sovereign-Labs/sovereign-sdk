mod inputs;
mod results;
mod roles;
/// Module including test case structs
mod tests;

pub use inputs::*;
pub use results::*;
pub use roles::*;
pub use tests::*;

/// A special configuration trait for types that need to be configured before they can be used.
/// Such types are typically constructed from state that cannot be known ahead of time.
pub trait FromState<S: sov_modules_api::Spec> {
    /// The type created by the [`FromState::from_state`] function.
    type Output;

    /// Executes the configuration logic and returns the configured output type.
    fn from_state(
        self: Box<Self>,
        state: &mut sov_modules_api::ApiStateAccessor<S>,
    ) -> Self::Output;
}
