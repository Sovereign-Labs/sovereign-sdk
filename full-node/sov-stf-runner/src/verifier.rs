use std::marker::PhantomData;

use sov_modules_api::Zkvm;
use sov_rollup_interface::da::DaVerifier;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmGuest;

use crate::StateTransitionData;

/// Verifies a state transition
pub struct StateTransitionVerifier<ST, Da, Zk>
where
    Da: DaVerifier,
    Zk: Zkvm,
    ST: StateTransitionFunction<Zk, Da::Spec>,
{
    app: ST,
    da_verifier: Da,
    phantom: PhantomData<Zk>,
}
impl<ST, Da, Zk> StateTransitionVerifier<ST, Da, Zk>
where
    Da: DaVerifier,
    Zk: ZkvmGuest,
    ST: StateTransitionFunction<Zk, Da::Spec>,
{
    /// Create a [`StateTransitionVerifier`]
    pub fn new(app: ST, da_verifier: Da) -> Self {
        Self {
            app,
            da_verifier,
            phantom: Default::default(),
        }
    }

    /// Verify the next block
    pub fn run_block(&mut self, zkvm: Zk) -> Result<ST::StateRoot, Da::Error> {
        let mut data: StateTransitionData<ST, Da::Spec, Zk> = zkvm.read_from_host();
        let validity_condition = self.da_verifier.verify_relevant_tx_list(
            &data.da_block_header,
            &data.blobs,
            data.inclusion_proof,
            data.completeness_proof,
        )?;

        let result = self.app.apply_slot(
            data.state_transition_witness,
            &data.da_block_header,
            &validity_condition,
            &mut data.blobs,
        );

        zkvm.commit(&result.state_root);
        Ok(result.state_root)
    }
}
