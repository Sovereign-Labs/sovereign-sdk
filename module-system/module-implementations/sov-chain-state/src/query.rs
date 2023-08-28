use sov_rollup_interface::da::DaSpec;
use sov_state::WorkingSet;

use super::ChainState;
use crate::{StateTransitionId, TransitionHeight, TransitionInProgress};

/// Structure returned by the query methods.
pub struct Response {
    /// Value returned by the queries
    pub value: u64,
}

impl<C: sov_modules_api::Context, Da: DaSpec> ChainState<C, Da> {
    /// Get the height of the current slot.
    /// Panics if the slot height is not set
    pub fn get_slot_height(&self, working_set: &mut WorkingSet<C::Storage>) -> TransitionHeight {
        self.slot_height
            .get(working_set)
            .expect("Slot height should be set at initialization")
    }

    /// Return the genesis hash of the module.
    pub fn get_genesis_hash(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<[u8; 32]> {
        self.genesis_hash.get(working_set)
    }

    /// Returns the transition in progress of the module.
    pub fn get_in_progress_transition(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<TransitionInProgress<Da>> {
        self.in_progress_transition.get(working_set)
    }

    /// Returns the completed transition associated with the provided `transition_num`.
    pub fn get_historical_transitions(
        &self,
        transition_num: TransitionHeight,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Option<StateTransitionId<Da>> {
        self.historical_transitions
            .get(&transition_num, working_set)
    }
}
