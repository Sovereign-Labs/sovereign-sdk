use alloy_eips::BlockId;
use alloy_primitives::hex::encode_prefixed;
use jsonrpsee_types::{
    error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    ErrorObject, ErrorObjectOwned,
};

/// Constructs a JSON-RPC error, consisting of `code`, `message` and optional `data`.
pub fn rpc_err(code: i32, msg: impl Into<String>, data: Option<&[u8]>) -> ErrorObjectOwned {
    ErrorObject::owned(
        code,
        msg.into(),
        data.map(|data| {
            jsonrpsee_core::to_json_raw_value(&encode_prefixed(data))
                .expect("serializing String can't fail")
        }),
    )
}

/// Constructs an internal JSON-RPC error with code and message
pub fn rpc_error_with_code(code: i32, msg: impl Into<String>) -> ErrorObjectOwned {
    rpc_err(code, msg, None)
}

/// Constructs an invalid params JSON-RPC error.
pub fn invalid_params_rpc_err(msg: impl Into<String>) -> ErrorObjectOwned {
    rpc_error_with_code(INVALID_PARAMS_CODE, msg)
}

/// Constructs an internal JSON-RPC error.
pub fn internal_rpc_err(msg: impl Into<String>) -> ErrorObjectOwned {
    rpc_error_with_code(INTERNAL_ERROR_CODE, msg)
}

/// Formats a [`BlockId`] into an error message.
pub fn block_id_to_str(id: BlockId) -> String {
    match id {
        BlockId::Hash(h) => {
            if h.require_canonical == Some(true) {
                format!("canonical hash {}", h.block_hash)
            } else {
                format!("hash {}", h.block_hash)
            }
        }
        BlockId::Number(n) => format!("{n}"),
    }
}
