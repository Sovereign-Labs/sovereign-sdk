use sov_modules_api::{hooks::SlotHooks, Context};
use sov_rollup_interface::zk::traits::{ValidityCondition, Zkvm};
use sov_state::WorkingSet;

use crate::ValidityConditions;

impl<Ctx: Context, Vm: Zkvm, Cond: ValidityCondition> SlotHooks
    for ValidityConditions<Ctx, Vm, Cond>
{
    type Context = Ctx;

    fn begin_slot_hook(
        &self,
        state_checkpoint: sov_state::StateCheckpoint<
            <Self::Context as sov_modules_api::Spec>::Storage,
        >,
    ) -> WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage> {
        let mut working_set = state_checkpoint.to_revertable();
        self.increment_slot_height(&mut working_set);
        working_set
    }
}
