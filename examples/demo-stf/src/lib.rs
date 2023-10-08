//#![deny(missing_docs)]
//#![doc = include_str!("../README.md")]

mod app;
pub use app::*;

#[cfg(feature = "native")]
pub mod genesis_config;
pub mod hooks_impl;
pub mod runtime;
#[cfg(test)]
pub mod tests;

#[cfg(feature = "native")]
pub mod cli;

use runtime::Runtime;
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::da::DaVerifier;
pub use sov_state::ArrayWitness;
use sov_stf_runner::verifier::StateTransitionVerifier;

/// A verifier for the demo rollup
pub type AppVerifier<DA, Zk> = StateTransitionVerifier<
    AppTemplate<
        ZkDefaultContext,
        <DA as DaVerifier>::Spec,
        Zk,
        Runtime<ZkDefaultContext, <DA as DaVerifier>::Spec>,
    >,
    DA,
    Zk,
>;
