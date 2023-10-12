#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "native")]
mod config;

#[cfg(feature = "native")]
use std::path::Path;

#[cfg(feature = "native")]
use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "native")]
pub use config::RpcConfig;
#[cfg(feature = "native")]
mod ledger_rpc;
#[cfg(feature = "native")]
mod runner;
#[cfg(feature = "native")]
pub use config::{from_toml_path, RollupConfig, RunnerConfig, StorageConfig};
#[cfg(feature = "native")]
pub use ledger_rpc::get_ledger_rpc;
#[cfg(feature = "native")]
pub use runner::*;
#[cfg(feature = "native")]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::Zkvm;

/// Implements the `StateTransitionVerifier` type for checking the validity of a state transition
pub mod verifier;

#[derive(Serialize, BorshDeserialize, BorshSerialize, Deserialize)]
// Prevent serde from generating spurious trait bounds. The correct serde bounds are already enforced by the
// StateTransitionFunction, DA, and Zkvm traits.
#[serde(bound = "")]
/// Data required to verify a state transition.
pub struct StateTransitionData<ST: StateTransitionFunction<Zk, DA>, DA: DaSpec, Zk>
where
    Zk: Zkvm,
{
    /// The state root before the state transition
    pub pre_state_root: ST::StateRoot,
    /// The header of the da block that is being processed
    pub da_block_header: DA::BlockHeader,
    /// The proof of inclusion for all blobs
    pub inclusion_proof: DA::InclusionMultiProof,
    /// The proof that the provided set of blobs is complete
    pub completeness_proof: DA::CompletenessProof,
    /// The blobs that are being processed
    pub blobs: Vec<<DA as DaSpec>::BlobTransaction>,
    /// The witness for the state transition
    pub state_transition_witness: ST::Witness,
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
