use super::ChainState;
use sov_rollup_interface::zk::traits::ValidityCondition;
use sov_state::WorkingSet;

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
}
