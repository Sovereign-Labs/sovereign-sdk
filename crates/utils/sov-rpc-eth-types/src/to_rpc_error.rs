use alloy_transport::{RpcError, TransportErrorKind};

use crate::utils::internal_rpc_err;

/// A trait to convert an error to an RPC error.
pub trait ToRpcError: core::error::Error + Send + Sync + 'static {
    /// Converts the error to a JSON-RPC error object.
    fn to_rpc_error(&self) -> jsonrpsee_types::ErrorObject<'static>;
}

impl ToRpcError for jsonrpsee_types::ErrorObject<'static> {
    fn to_rpc_error(&self) -> jsonrpsee_types::ErrorObject<'static> {
        self.clone()
    }
}

impl ToRpcError for RpcError<TransportErrorKind> {
    fn to_rpc_error(&self) -> jsonrpsee_types::ErrorObject<'static> {
        match self {
            Self::ErrorResp(payload) => jsonrpsee_types::error::ErrorObject::owned(
                payload.code as i32,
                payload.message.clone(),
                payload.data.clone(),
            ),
            err => internal_rpc_err(err.to_string()),
        }
    }
}
