use super::ValueSetter;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

#[derive(BorshDeserialize, BorshSerialize)]
pub enum QueryMessage {
    GetValue,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: Option<u32>,
}

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Queries the state of the module.
    pub fn query_value(&self) -> Response {
        Response {
            value: self.value.get(),
        }
    }
}
