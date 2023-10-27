#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
pub mod genesis_config;
mod hooks_impl;
pub mod runtime;
#[cfg(test)]
mod tests;

use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::da::DaVerifier;
use sov_stf_runner::verifier::StateTransitionVerifier;

/// Alias for StateTransitionVerifier.
pub type AppVerifier<DA, Vm, ZkContext, RT, K> =
    StateTransitionVerifier<AppTemplate<ZkContext, <DA as DaVerifier>::Spec, Vm, RT, K>, DA, Vm>;
