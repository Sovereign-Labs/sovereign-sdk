mod parallel;

mod block_proof;

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use borsh::BorshSerialize;
pub use parallel::ParallelProverService;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::{ZkVerifier, Zkvm, ZkvmHost};
use strum::{Display, EnumString};
use thiserror::Error;

pub use crate::processes::StateTransitionInfo;

/// The possible configurations of the prover
// We use arcs for cheap cloning
#[derive(Clone)]
pub enum RollupProverConfig<Vm: Zkvm> {
    /// Skip proving.
    Skip,
    /// Run the rollup verifier in a zkVM executor.
    Execute(Arc<<Vm::Host as ZkvmHost>::HostArgs>),
    /// Run the rollup verifier and create a SNARK of execution.
    Prove(Arc<<Vm::Host as ZkvmHost>::HostArgs>),
}

/// The associated discriminants of [`RollupProverConfig`]. Possible configurations of the prover
// Note: it's best if all string conversions to and from this type (even
// `Debug`) use the same casing, to avoid bad UX or confusion around env. vars
// expected behavior.
#[derive(Clone, Copy, PartialEq, Eq, EnumString, Display)]
#[strum(serialize_all = "snake_case")]
pub enum RollupProverConfigDiscriminants {
    /// Skip proving.
    Skip,
    /// Run the rollup verifier in a zkVM executor.
    Execute,
    /// Run the rollup verifier and create a SNARK of execution.
    Prove,
}

impl Debug for RollupProverConfigDiscriminants {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

impl<Vm: Zkvm> From<RollupProverConfig<Vm>> for RollupProverConfigDiscriminants {
    fn from(value: RollupProverConfig<Vm>) -> Self {
        match value {
            RollupProverConfig::Prove(_) => RollupProverConfigDiscriminants::Prove,
            RollupProverConfig::Execute(_) => RollupProverConfigDiscriminants::Execute,
            RollupProverConfig::Skip => RollupProverConfigDiscriminants::Skip,
        }
    }
}

impl RollupProverConfigDiscriminants {
    /// Converts the discriminant into a config
    pub fn into_config<Vm: Zkvm>(
        self,
        host_args: Arc<<Vm::Host as ZkvmHost>::HostArgs>,
    ) -> RollupProverConfig<Vm> {
        match self {
            RollupProverConfigDiscriminants::Skip => RollupProverConfig::Skip,
            RollupProverConfigDiscriminants::Execute => RollupProverConfig::Execute(host_args),
            RollupProverConfigDiscriminants::Prove => RollupProverConfig::Prove(host_args),
        }
    }
}

impl<Vm: Zkvm> RollupProverConfig<Vm> {
    /// Splits the rollup prover config into host arguments and an associated discriminant
    pub fn split(
        self,
    ) -> (
        Arc<<Vm::Host as ZkvmHost>::HostArgs>,
        RollupProverConfigDiscriminants,
    ) {
        match self {
            RollupProverConfig::Skip => (Default::default(), RollupProverConfigDiscriminants::Skip),
            RollupProverConfig::Execute(host_args) => {
                (host_args, RollupProverConfigDiscriminants::Execute)
            }
            RollupProverConfig::Prove(host_args) => {
                (host_args, RollupProverConfigDiscriminants::Prove)
            }
        }
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
        block_header_hashes: &[<<Self::DaService as DaService>::Spec as DaSpec>::SlotHash],
        genesis_state_root: &Self::StateRoot,
    ) -> anyhow::Result<ProofAggregationStatus>;
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn prover_config_debug_and_display_are_the_same() {
        let config = RollupProverConfigDiscriminants::Skip;
        assert_eq!(format!("{config:?}"), format!("{}", config));
    }

    #[test]
    fn prover_config_display_from_str() {
        let config = RollupProverConfigDiscriminants::Skip;
        assert_eq!(
            RollupProverConfigDiscriminants::from_str(&config.to_string()).unwrap(),
            config
        );
    }
}
