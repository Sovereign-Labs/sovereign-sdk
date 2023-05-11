use super::ExampleModule;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_state::WorkingSet;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage {
    GetValue,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: Option<u32>,
}

impl<C: sov_modules_api::Context> ExampleModule<C> {
    /// Queries the state of the module.
    pub fn query_value(&self, working_set: &mut WorkingSet<C::Storage>) -> Response {
        Response {
            value: self.value.get(working_set),
        }
    }
}
