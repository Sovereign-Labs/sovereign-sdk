use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_state::WorkingSet;

use super::UnprovableSetter;

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct ValueResponse {
    pub value: Option<String>,
}

#[rpc_gen(client, server, namespace = "unprovable_setter")]
impl<C: sov_modules_api::Context> UnprovableSetter<C> {
    #[rpc_method(name = "value")]
    pub fn query_value(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<ValueResponse> {
        Ok(ValueResponse {
            value: self.get_value_unprovable(working_set),
        })
    }
}
