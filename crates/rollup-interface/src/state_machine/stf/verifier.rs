use std::marker::PhantomData;

use crate::da::{BlockHeaderTrait, DaVerifier};
use crate::stf::{ExecutionContext, StateTransitionFunction};
use crate::zk::{StateTransitionPublicData, StateTransitionWitnessWithAddress, Zkvm, ZkvmGuest};

/// Verifies a state transition.
pub struct StateTransitionVerifier<ST, Da, InnerVm, OuterVm>
where
    Da: DaVerifier,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    ST: StateTransitionFunction<InnerVm, OuterVm, Da::Spec>,
{
    app: ST,
    da_verifier: Da,
    phantom: PhantomData<(InnerVm, OuterVm)>,
}

impl<Stf, Da, InnerVm, OuterVm> StateTransitionVerifier<Stf, Da, InnerVm, OuterVm>
where
    Da: DaVerifier,
    InnerVm: Zkvm,
    OuterVm: Zkvm,
    Stf: StateTransitionFunction<InnerVm, OuterVm, Da::Spec>,
{
    /// Create a [`StateTransitionVerifier`]
    pub fn new(app: Stf, da_verifier: Da) -> Self {
        Self {
            app,
            da_verifier,
            phantom: Default::default(),
        }
    }

    /// Verify the next block
    pub fn run_block(
        &self,
        zkvm: InnerVm::Guest,
        pre_state: Stf::PreState,
    ) -> Result<(), Da::Error> {
        let data: StateTransitionWitnessWithAddress<Stf::Address, _, _, Da::Spec> =
            zkvm.read_from_host();

        let prover_address = data.prover_address;
        let mut data = data.stf_witness;

        self.da_verifier.verify_relevant_tx_list(
            &data.da_block_header,
            &data.relevant_blobs,
            data.relevant_proofs,
        )?;

        let result = self.app.apply_slot(
            &data.initial_state_root,
            pre_state,
            data.witness,
            &data.da_block_header,
            data.relevant_blobs.as_iters(),
            ExecutionContext::Node,
        );

        let out: StateTransitionPublicData<Stf::Address, Da::Spec, _> = StateTransitionPublicData {
            initial_state_root: data.initial_state_root,
            final_state_root: result.state_root,
            slot_hash: data.da_block_header.hash(),
            prover_address,
        };

        zkvm.commit(&out);
        Ok(())
    }
}
