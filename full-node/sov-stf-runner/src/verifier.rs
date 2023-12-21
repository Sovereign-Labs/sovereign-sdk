use std::marker::PhantomData;

use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlockHeaderTrait, DaVerifier};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::{StateTransition, Zkvm, ZkvmGuest};

use crate::StateTransitionData;

#[derive(Serialize, Deserialize)]
/// Output of the verifier.
pub struct StateTransitionOutput<StateRoot, SlotHash> {
    /// The state root before the state transition
    pub pre_state_root: StateRoot,
    /// The state root after the state transition
    pub post_state_root: StateRoot,
    /// Da block hash
    pub da_block_hash: SlotHash,
    /// The block height.
    pub height: u64,
}

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

impl<Stf, Da, Zk> StateTransitionVerifier<Stf, Da, Zk>
where
    Da: DaVerifier,
    Zk: ZkvmGuest,
    Stf: StateTransitionFunction<Zk, Da::Spec>,
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
    pub fn run_block(&self, zkvm: Zk, pre_state: Stf::PreState) -> Result<(), Da::Error> {
        let mut data: StateTransitionData<_, _, Da::Spec> = zkvm.read_from_host();
        let validity_condition = self.da_verifier.verify_relevant_tx_list(
            &data.da_block_header,
            &data.blobs,
            data.inclusion_proof,
            data.completeness_proof,
        )?;

        let result = self.app.apply_slot(
            &data.pre_state_root,
            pre_state,
            data.state_transition_witness,
            &data.da_block_header,
            &validity_condition,
            &mut data.blobs,
        );

        /*
        let out = StateTransitionOutput {
            pre_state_root: data.pre_state_root,
            post_state_root: result.state_root,
            da_block_hash: data.da_block_header.hash(),
            height: data.da_block_header.height(),
        };*/

        let out: StateTransition<Da::Spec, Vec<u8>, _> = StateTransition {
            initial_state_root: data.pre_state_root,
            final_state_root: result.state_root,
            slot_hash: data.da_block_header.hash(),
            rewarded_address: Vec::default(),
            validity_condition,
        };

        zkvm.commit(&out);
        Ok(())
    }
}
