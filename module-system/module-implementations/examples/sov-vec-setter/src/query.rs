use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::WorkingSet;

use super::VecSetter;

/// Response returned from the vecSetter_queryVec endpoint.
#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct QueryResponse {
    /// Value saved in the module's state vector.
    pub value: Option<u32>,
}

/// Response returned from the vecSetter_lenVec endpoint
#[derive(serde::Serialize, serde::Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct LenResponse {
    /// Length of the vector
    pub value: usize,
}

#[rpc_gen(client, server, namespace = "vecSetter")]
impl<C: sov_modules_api::Context> VecSetter<C> {
    /// Queries the state vector of the module.
    #[rpc_method(name = "queryVec")]
    pub fn query_vec(
        &self,
        index: usize,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<QueryResponse> {
        Ok(QueryResponse {
            value: self.vector.get(index, working_set),
        })
    }
    /// Queries the length of the vector
    #[rpc_method(name = "lenVec")]
    pub fn len_vec(&self, working_set: &mut WorkingSet<C>) -> RpcResult<LenResponse> {
        Ok(LenResponse {
            value: self.vector.len(working_set),
        })
    }
}
