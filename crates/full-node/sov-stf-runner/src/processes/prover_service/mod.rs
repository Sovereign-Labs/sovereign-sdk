mod network;
mod parallel;

use std::sync::Arc;
use std::fmt::Debug;

use async_trait::async_trait;
use borsh::BorshSerialize;
pub use network::NetworkProverService;
pub use parallel::ParallelProverService;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::ZkVerifier;
use strum::{Display, EnumString};
use thiserror::Error;
use tokio::sync::Notify;

pub use crate::processes::StateTransitionInfo;

pub(crate) struct Verifier<Da>
where
    Da: DaService,
{
    pub(crate) da_verifier: Da::Verifier,
}

/// Prover mode, parsed from the `SOV_PROVER_MODE` env var / CLI flag.
///
/// Blueprints that need more configuration (e.g., which guest ELF to prove) should source
/// that directly in [`crate::processes::ProverService`] construction rather than threading
/// it through this type.
#[derive(Clone, Copy, PartialEq, Eq, EnumString, Display)]
#[strum(serialize_all = "snake_case")]
pub enum RollupProverConfig {
    /// Run the rollup verifier and create a SNARK of execution.
    Prove,
    /// Proving is disabled.
    Disabled,
}

impl RollupProverConfig {
    /// Returns `true` unless the prover is disabled.
    pub fn is_enabled(&self) -> bool {
        !matches!(self, RollupProverConfig::Disabled)
    }
}

impl Debug for RollupProverConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

/// Represents the status of a witness submission.
#[derive(Debug, Eq, PartialEq)]
pub enum WitnessSubmissionStatus {
    /// The witness has been submitted to the prover.
    SubmittedForProving,
    /// The witness is already present in the prover.
    WitnessExist,
}

/// Represents the status of a DA proof submission.
#[derive(Debug, Eq, PartialEq)]
pub enum ProofAggregationStatus {
    /// Indicates successful proof generation.
    Success(SerializedAggregatedProof),
    /// Indicates that proof generation is currently in progress.
    ProofGenerationInProgress,
}

/// Represents the current status of proof generation.
#[derive(derivative::Derivative)]
#[derivative(Debug(bound = ""))]
pub enum ProofProcessingStatus<StateRoot, Witness, Da: DaSpec> {
    /// Indicates that proof generation is currently in progress.
    ProvingInProgress,
    /// Indicates that the prover is busy and will not initiate a new proving process.
    /// Returns the witness data that was provided by the caller.
    Busy(#[derivative(Debug = "ignore")] StateTransitionInfo<StateRoot, Witness, Da>),
}

/// An error that occurred during ZKP proving.
#[derive(Error, Debug)]
pub enum ProverServiceError {
    /// The prover is too busy to take on any additional jobs at the moment.
    #[error("Prover is too busy")]
    ProverBusy,
    /// Some internal prover error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// This service is responsible for ZK proof generation.
/// The proof generation process involves the following stages:
///     1. Submitting a witness using the `submit_witness` method to a prover service.
///     2. Initiating proof generation with the `prove` method.
/// Once the proof is ready, it can be sent to the DA with `send_proof_to_da` method.
/// Currently, the cancellation of proving jobs for submitted witnesses is not supported,
/// but this functionality will be added in the future (#1185).
#[async_trait]
pub trait ProverService: Send + Sync + 'static {
    /// Ths root hash of state merkle tree.
    type StateRoot: BorshSerialize
        + Serialize
        + DeserializeOwned
        + Clone
        + AsRef<[u8]>
        + Send
        + Sync
        + 'static;
    /// Data that is produced during batch execution.
    type Witness: Serialize + DeserializeOwned + Send + Sync;
    /// Data Availability service.
    type DaService: DaService;

    /// Verifier for the aggregated proof.
    type Verifier: ZkVerifier;

    /// Creates ZK proof for a block corresponding to `block_header_hash`.
    async fn prove(
        &self,
        state_transition_info: StateTransitionInfo<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    ) -> Result<
        ProofProcessingStatus<Self::StateRoot, Self::Witness, <Self::DaService as DaService>::Spec>,
        ProverServiceError,
    >;

    /// Sends the ZK proof to the DA.
    /// This method is not yet fully implemented: see #1185
    async fn create_aggregated_proof(
        &self,
        block_headers: &[<<Self::DaService as DaService>::Spec as DaSpec>::BlockHeader],
        genesis_state_root: &Self::StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus>;

    /// Notifies when the prover may be able to accept more proving work.
    fn capacity_notify(&self) -> Option<Arc<Notify>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn prover_config_debug_and_display_are_the_same() {
        let config = RollupProverConfig::Prove;
        assert_eq!(format!("{config:?}"), format!("{}", config));
    }

    #[test]
    fn prover_config_display_from_str() {
        let config = RollupProverConfig::Prove;
        assert_eq!(
            RollupProverConfig::from_str(&config.to_string()).unwrap(),
            config
        );
    }
}
