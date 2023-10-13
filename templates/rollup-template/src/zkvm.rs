//! This module selects the ZKVM to be used to prove the rollup.
//! To change ZKVMs:
//!   1. Switch the `sov_risc0_adapter` dependency in you Cargo.toml to the adapter for your chosen ZKVM
//!   2. Update the two type aliases in this file

#[cfg(feature = "native")]
use sov_risc0_adapter::host::Risc0Host;

/// The type alias for the host ("prover").
#[cfg(feature = "native")]
pub type ZkvmHost = Risc0Host<'static>;
