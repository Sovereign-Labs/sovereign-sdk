use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;

use super::ValueSetter;

#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: Option<u32>,
}

#[rpc_gen(client, server, namespace = "valueSetter")]
impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Queries the state of the module.
    #[rpc_method(name = "queryValue")]
    pub fn query_value(&self, working_set: &mut WorkingSet<C::Storage>) -> Response {
        Response {
            value: self.value.get(working_set),
        }
    }
}
