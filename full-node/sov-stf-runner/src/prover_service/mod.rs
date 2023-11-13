mod single_threaded_prover;

use crate::StateTransitionData;
use async_trait::async_trait;
use serde::Serialize;
use sov_rollup_interface::{da::DaSpec, services::da::DaService};
use thiserror::Error;

pub(crate) type Hash = [u8; 32];

#[derive(Error, Debug)]
pub enum ProverServiceError {
    #[error("ProverBusy error")]
    ProverBusy,
    #[error("Other error")]
    Other(#[from] anyhow::Error),
}

#[async_trait]
pub trait ProverService {
    type StateRoot: Serialize + Clone + AsRef<[u8]>;
    type Witness: Serialize;
    type DaService: DaService;

    fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    );

    async fn prove(&self, block_header_hash: Hash) -> Result<(), ProverServiceError>;
}
