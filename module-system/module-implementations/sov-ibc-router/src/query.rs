use sov_state::WorkingSet;

use super::IbcRouter;

#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: Option<u32>,
}

impl<C: sov_modules_api::Context> IbcRouter<C> {
    /// Queries the state of the module.
    pub fn query_value(&self, working_set: &mut WorkingSet<C::Storage>) -> Response {
        Response {
            value: self.value.get(working_set),
        }
    }
}
