use crate::da::{BlockHeaderTrait, DaVerifier};
use crate::stf::{ExecutionContext, StateTransitionFunction};
use crate::zk::{StateTransitionPublicData, StateTransitionWitnessWithAddress, ZkvmGuest};

/// Verifies a state transition.
pub struct StateTransitionVerifier<ST, Da>
where
    Da: DaVerifier,
    ST: StateTransitionFunction<Da::Spec>,
{
    app: ST,
    da_verifier: Da,
}

impl<Stf, Da> StateTransitionVerifier<Stf, Da>
where
    Da: DaVerifier,
    Stf: StateTransitionFunction<Da::Spec>,
{
    /// Create a [`StateTransitionVerifier`]
    pub fn new(app: Stf, da_verifier: Da) -> Self {
        Self { app, da_verifier }
    }

    /// Verify the next block
    pub fn run_block<G: ZkvmGuest>(
        &self,
        zkvm: G,
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
