#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
mod config;
#[cfg(feature = "mock")]
/// Testing utilities.
#[cfg(feature = "mock")]
pub mod mock;
#[cfg(feature = "native")]
mod prover_service;

#[cfg(feature = "native")]
use std::path::Path;

#[cfg(feature = "native")]
use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "native")]
pub use config::RpcConfig;
#[cfg(feature = "native")]
pub use prover_service::*;
#[cfg(feature = "native")]
mod runner;
#[cfg(feature = "native")]
pub use config::{from_toml_path, RollupConfig, RunnerConfig, StorageConfig};
#[cfg(feature = "native")]
pub use runner::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::DaSpec;

/// Implements the `StateTransitionVerifier` type for checking the validity of a state transition
pub mod verifier;

#[derive(Serialize, BorshDeserialize, BorshSerialize, Deserialize)]
// Prevent serde from generating spurious trait bounds. The correct serde bounds are already enforced by the
// StateTransitionFunction, DA, and Zkvm traits.
#[serde(bound = "StateRoot: Serialize + DeserializeOwned, Witness: Serialize + DeserializeOwned")]
/// Data required to verify a state transition.
pub struct StateTransitionData<StateRoot, Witness, Da: DaSpec> {
    /// The state root before the state transition
    pub pre_state_root: StateRoot,
    /// The header of the da block that is being processed
    pub da_block_header: Da::BlockHeader,
    /// The proof of inclusion for all blobs
    pub inclusion_proof: Da::InclusionMultiProof,
    /// The proof that the provided set of blobs is complete
    pub completeness_proof: Da::CompletenessProof,
    /// The blobs that are being processed
    pub blobs: Vec<<Da as DaSpec>::BlobTransaction>,
    /// The witness for the state transition
    pub state_transition_witness: Witness,
}

#[cfg(feature = "native")]
/// Reads json file.
pub fn read_json_file<T: DeserializeOwned, P: AsRef<Path>>(path: P) -> anyhow::Result<T> {
    let path_str = path.as_ref().display();

    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read genesis from {}", path_str))?;
    let config: T = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse genesis from {}", path_str))?;

    Ok(config)
}
