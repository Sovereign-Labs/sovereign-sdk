use sov_rollup_interface::zk::ValidityCondition;
use sov_state::WorkingSet;

use super::ChainState;

#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq)]
/// Structure returned by the query methods.
pub struct Response {
    /// Value returned by the queries
    pub value: u64,
}

impl<C: sov_modules_api::Context, Cond: ValidityCondition> ChainState<C, Cond> {
    /// Get the height of the current slot
    pub fn slot_height(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<u64> {
        self.slot_height.get(working_set)
    }

    /// Return the genesis hash of the module.
    pub fn genesis_hash(&self, working_set: &mut WorkingSet<C::Storage>) -> Option<[u8; 32]> {
        self.genesis_hash.get(working_set)
    }
}
