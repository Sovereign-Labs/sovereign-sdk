use super::{ProverService, ProverServiceError};
use crate::StateTransitionData;
use async_trait::async_trait;
use serde::Serialize;
use sov_rollup_interface::services::da::DaService;
use std::marker::PhantomData;

pub struct SimpleProver<StateRoot, Witness, Da>
where
    StateRoot: Serialize + Clone + AsRef<[u8]>,
    Witness: Serialize,
    Da: DaService,
{
    _p_state_root: PhantomData<StateRoot>,
    _p_witness: PhantomData<Witness>,
    _p_da: PhantomData<Da>,
}

#[async_trait]
impl<StateRoot, Witness, Da> ProverService for SimpleProver<StateRoot, Witness, Da>
where
    StateRoot: Serialize + Clone + AsRef<[u8]> + Send + Sync,
    Witness: Serialize + Send + Sync,
    Da: DaService,
{
    type StateRoot = StateRoot;

    type Witness = Witness;

    type DaService = Da;

    fn submit_witness(
        &self,
        state_transition_data: StateTransitionData<
            Self::StateRoot,
            Self::Witness,
            <Self::DaService as DaService>::Spec,
        >,
    ) {
        todo!()
    }

    async fn prove(&self, state_root: Self::StateRoot) -> Result<(), ProverServiceError> {
        todo!()
    }
}
