#![allow(missing_docs)]
use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_state::WorkingSet;

use super::ValueSetter;

/// Response returned from the valueSetter_queryValue endpoint.
#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct Response {
    /// Value saved in the module's state.
    pub value: Option<String>,
}

#[rpc_gen(client, server, namespace = "valueSetter")]
impl<C: sov_modules_api::Context, D> ValueSetter<C, D>
where
    D: std::hash::Hash
        + std::clone::Clone
        + borsh::BorshSerialize
        + borsh::BorshDeserialize
        + std::fmt::Debug
        + std::str::FromStr,
{
    /// Queries the state of the module.
    #[rpc_method(name = "queryValue")]
    pub fn query_value(&self, working_set: &mut WorkingSet<C::Storage>) -> RpcResult<Response> {
        let value = self.value.get(working_set).map(|v| format!("{:?}", v));
        Ok(Response { value })
    }
}
