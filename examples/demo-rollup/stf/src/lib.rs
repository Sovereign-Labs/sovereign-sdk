#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
pub mod genesis_config;
pub mod runtime;
#[cfg(feature = "test-utils")]
mod test_utils;

use sov_modules_stf_blueprint::StfBlueprint;
use sov_rollup_interface::stf::StateTransitionVerifier;

/// Alias for StateTransitionVerifier.
pub type StfVerifier<DA, ZkSpec, RT, InnerVm, OuterVm> =
    StateTransitionVerifier<StfBlueprint<ZkSpec, RT>, DA, InnerVm, OuterVm>;
