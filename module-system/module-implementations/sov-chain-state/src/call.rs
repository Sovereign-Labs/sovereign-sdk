use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::zk::ValidityCondition;

use sov_state::WorkingSet;

use crate::{ChainState, StateTransitionId, TransitionHeight};

impl<
        Ctx: sov_modules_api::Context,
        Cond: ValidityCondition + BorshSerialize + BorshDeserialize,
    > ChainState<Ctx, Cond>
{
    /// Increment the current slot height
    pub fn increment_slot_height(&self, working_set: &mut WorkingSet<Ctx::Storage>) {
        let current_height = self
            .slot_height
            .get(working_set)
            .expect("Block height must be initialized");
        self.slot_height.set(
            &TransitionHeight(current_height.inner().saturating_add(1)),
            working_set,
        );
    }

    /// Store the previous state transition
    pub fn store_state_transition(
        &self,
        height: TransitionHeight,
        transition: StateTransitionId<Cond>,
        working_set: &mut WorkingSet<Ctx::Storage>,
    ) {
        self.historical_transitions
            .set(&height, &transition, working_set);
    }
}
