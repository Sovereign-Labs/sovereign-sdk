use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::{hooks::SlotHooks, Context};
use sov_rollup_interface::{
    stf::RollupHeaderTrait,
    zk::traits::{StateTransition, ValidityCondition},
};
use sov_state::WorkingSet;

use crate::ChainState;

impl<Ctx: Context, Cond: ValidityCondition + BorshDeserialize + BorshSerialize> SlotHooks
    for ChainState<Ctx, Cond>
{
    type Context = Ctx;

    fn begin_slot_hook(
        &self,
        state_checkpoint: sov_state::StateCheckpoint<
            <Self::Context as sov_modules_api::Spec>::Storage,
        >,
        rollup_header: impl RollupHeaderTrait,
    ) -> WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage> {
        let mut working_set = state_checkpoint.to_revertable();
        self.increment_slot_height(&mut working_set);
        // TODO: store state transition
        working_set
    }
}
