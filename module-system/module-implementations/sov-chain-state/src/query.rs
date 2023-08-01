use sov_rollup_interface::zk::ValidityCondition;
use sov_state::WorkingSet;

use super::ChainState;

#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: u64,
}

impl<C: sov_modules_api::Context, Cond: ValidityCondition> ChainState<C, Cond> {
    /// Get the height of the current slot
    pub fn slot_height(&self, working_set: &mut WorkingSet<C::Storage>) -> u64 {
        self.slot_height
            .get(working_set)
            .expect("Block height must be set")
    }

    pub fn genesis_hash(&self, working_set: &mut WorkingSet<C::Storage>) -> [u8; 32] {
        self.historical_transitions
            .get(&0, working_set)
            .expect("Genesis hash must be set")
            .post_state_root
    }
}
