#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
mod native_only;
#[cfg(feature = "native")]
pub use native_only::*;
pub mod pluggable_traits;

use pluggable_traits::PluggableSpec;
use sov_modules_api::capabilities::HasCapabilities;
use sov_modules_api::execution_mode::ExecutionMode;
use sov_modules_api::Spec;
use sov_modules_stf_blueprint::Runtime;

/// A trait defining the logical STF of the rollup.
pub trait RollupBlueprint<M: ExecutionMode>: Sized + Send + Sync + 'static {
    /// The types provided by the rollup
    type Spec: PluggableSpec + Spec;

    /// The runtime for the rollup.
    type Runtime: Runtime<Self::Spec> + HasCapabilities<Self::Spec> + Send + Sync + 'static;
}
