//! The rollup State Transition Function.
#[cfg(feature = "native")]
mod builder;
mod hooks;
mod runtime;

#[cfg(feature = "native")]
pub(crate) use builder::*;
pub use runtime::*;
