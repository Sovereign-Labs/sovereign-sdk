use sov_rollup_interface::da::DaSpec;
use sov_state::WorkingSet;

use crate::{ChainState, StateTransitionId, TransitionHeight};

impl<Ctx: sov_modules_api::Context, Da: DaSpec> ChainState<Ctx, Da> {
    /// Increment the current slot height
    pub fn increment_slot_height(&self, working_set: &mut WorkingSet<Ctx::Storage>) {
        let current_height = self
            .slot_height
            .get(working_set)
            .expect("Block height must be initialized");
        self.slot_height
            .set(&(current_height.saturating_add(1)), working_set);
    }

    /// Store the previous state transition
    pub fn store_state_transition(
        &self,
        height: TransitionHeight,
        transition: StateTransitionId<Da>,
        working_set: &mut WorkingSet<Ctx::Storage>,
    ) {
        self.historical_transitions
            .set(&height, &transition, working_set);
    }
}
