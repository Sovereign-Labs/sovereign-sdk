mod parallel;
use async_trait::async_trait;
pub use parallel::ParallelProverService;
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use thiserror::Error;

use crate::StateTransitionData;

/// The possible configurations of the prover.
pub enum RollupProverConfig {
    /// Skip proving.
    Skip,
    /// Run the rollup verification logic inside the current process
    Simulate,
    /// Run the rollup verifier in a zkVM executor
    Execute,
    /// Run the rollup verifier and create a SNARK of execution
    Prove,
}

/// Represents the status of a witness submission.
#[derive(Debug, Eq, PartialEq)]
pub enum WitnessSubmissionStatus {
    ///
    SubmittedForProving,
    ///
    WitnessExist,
}

/// Represents the status of a DA proof submission.
#[derive(Debug, Eq, PartialEq)]
pub enum ProofSubmissionStatus {
    /// Indicates successful submission of the proof to the DA.
    Success,
    /// Indicates that proof generation is currently in progress.
    ProofGenerationInProgress,
}

/// Represents the current status of proof generation.
#[derive(Debug, Eq, PartialEq)]
pub enum ProofProcessingStatus {
    /// Indicates that proof generation is currently in progress.
    ProvingInProgress,
    /// Indicates that the prover is busy and will not initiate a new proving process.
    Busy,
}

/// An error that occurred during ZKP proving.
#[derive(Error, Debug)]
pub enum ProverServiceError {
    /// Prover is too busy.
    #[error("Prover is too busy")]
    ProverBusy,
    /// Some internal prover error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// This service is responsible for ZKP proof generation.
/// The proof generation process involves two stages:
/// 1. Submitting a witness using the `submit_witness` method to a prover service.
/// 2. Triggering proof generation with the `prove` method.

#[async_trait]
pub trait ProverService {
    /// Ths root hash of state merkle tree.
    type StateRoot: Serialize + Clone + AsRef<[u8]>;
    /// Data that is produced during batch execution.
    type Witness: Serialize;
    /// Data Availability service.
    type DaService: DaService;

    /// Submit a witness for proving.
    async fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    ) -> WitnessSubmissionStatus;

    /// Creates ZKP prove for a block corresponding to `block_header_hash`.
    async fn prove(
        &self,
        block_header_hash: <<Self::DaService as DaService>::Spec as DaSpec>::SlotHash,
    ) -> Result<ProofProcessingStatus, ProverServiceError>;

    /// Sends the ZK proof to the DA.
    async fn send_proof_to_da(
        &self,
        block_header_hash: <<Self::DaService as DaService>::Spec as DaSpec>::SlotHash,
    ) -> Result<ProofSubmissionStatus, anyhow::Error>;
}
