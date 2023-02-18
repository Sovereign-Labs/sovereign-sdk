use super::ValueAdderModule;
use borsh::BorshDeserialize;
use serde::{Deserialize, Serialize};

#[derive(BorshDeserialize)]
pub enum QueryMessage {
    GetValue,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: Option<u32>,
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    /// Queries the state of the module.
    pub fn query_value(&self) -> Response {
        Response {
            value: self.value.get(),
        }
    }
}
