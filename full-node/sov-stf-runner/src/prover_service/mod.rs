mod single_threaded_prover;

use crate::StateTransitionData;
use async_trait::async_trait;
use serde::Serialize;
use sov_rollup_interface::{da::DaSpec, services::da::DaService};
use thiserror::Error;

pub(crate) type Hash = [u8; 32];

/// The possible configurations of the prover.
pub enum RollupProverConfig {
    /// Run the rollup verification logic inside the current process
    Simulate,
    /// Run the rollup verifier in a zkVM executor
    Execute,
    /// Run the rollup verifier and create a SNARK of execution
    Prove,
}

///TODO
#[derive(Error, Debug)]
pub enum ProverServiceError {
    ///TODO
    #[error("ProverBusy error")]
    ProverBusy,
    ///TODO
    #[error("Other error")]
    Other(#[from] anyhow::Error),
}

/// TODO
#[async_trait]
pub trait ProverService {
    /// TODO
    type StateRoot: Serialize + Clone + AsRef<[u8]>;
    /// TODO
    type Witness: Serialize;
    /// TODO
    type DaService: DaService;

    /// TODO
    fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    );

    /// TODO
    async fn prove(&self, block_header_hash: Hash) -> Result<(), ProverServiceError>;
}
