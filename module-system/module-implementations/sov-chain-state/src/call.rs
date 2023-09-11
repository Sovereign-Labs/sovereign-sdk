use sov_state::WorkingSet;

use crate::{ChainState, StateTransitionId, TransitionHeight};

impl<C, Da> ChainState<C, Da>
where
    C: sov_modules_api::Context,
    Da: sov_modules_api::DaSpec,
{
    /// Increment the current slot height
    pub(crate) fn increment_slot_height(&self, working_set: &mut WorkingSet<C::Storage>) {
        let current_height = self
            .slot_height
            .get(working_set)
            .expect("Block height must be initialized");
        self.slot_height
            .set(&(current_height.saturating_add(1)), working_set);
    }

    /// Store the previous state transition
    pub(crate) fn store_state_transition(
        &self,
        height: TransitionHeight,
        transition: StateTransitionId<Da>,
        working_set: &mut WorkingSet<C::Storage>,
    ) {
        self.historical_transitions
            .set(&height, &transition, working_set);
    }
}
