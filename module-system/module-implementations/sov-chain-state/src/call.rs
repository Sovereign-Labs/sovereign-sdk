use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::zk::ValidityCondition;
use sov_state::WorkingSet;

use crate::{ChainState, StateTransitionId, TransitionHeight};

impl<C, Cond> ChainState<C, Cond>
where
    C: sov_modules_api::Context,
    Cond: ValidityCondition + BorshSerialize + BorshDeserialize,
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
        transition: StateTransitionId<Cond>,
        working_set: &mut WorkingSet<C::Storage>,
    ) {
        self.historical_transitions
            .set(&height, &transition, working_set);
    }
}
