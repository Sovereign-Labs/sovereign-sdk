//! JSON-RPC server implementation for the [`AccessorySetter`] module.

use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_state::WorkingSet;

use super::AccessorySetter;

/// Response type to the `accessorySetter_value` endpoint.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct ValueResponse {
    /// The value stored in the accessory state.
    pub value: Option<String>,
}

#[rpc_gen(client, server, namespace = "accessorySetter")]
impl<C: sov_modules_api::Context> AccessorySetter<C> {
    /// Returns the latest value set in the accessory state via
    /// [`CallMessage::SetValueAccessory`](crate::CallMessage::SetValueAccessory).
    #[rpc_method(name = "value")]
    pub fn query_value(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> RpcResult<ValueResponse> {
        Ok(ValueResponse {
            value: self.accessory_value.get(&mut working_set.accessory_state()),
        })
    }
}
