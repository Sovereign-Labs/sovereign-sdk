//! Additional helpers for converting errors.

use std::fmt::Display;

use jsonrpsee::core::RpcResult;

/// Helper trait to easily convert various `Result` types into [`RpcResult`]
pub(crate) trait ToRpcResult<Ok, Err> {
    /// Converts the error of the [Result] to an [RpcResult] via the `Err` [Display] impl.
    fn to_rpc_result(self) -> RpcResult<Ok>
    where
        Err: Display,
        Self: Sized,
    {
        self.map_internal_err(|err| err.to_string())
    }

    /// Converts this type into an [`RpcResult`]
    fn map_rpc_err<'a, F, M>(self, op: F) -> RpcResult<Ok>
    where
        F: FnOnce(Err) -> (i32, M, Option<&'a [u8]>),
        M: Into<String>;

    /// Converts this type into an [`RpcResult`] with the
    /// [`jsonrpsee::types::error::INTERNAL_ERROR_CODE` and the given message.
    fn map_internal_err<F, M>(self, op: F) -> RpcResult<Ok>
    where
        F: FnOnce(Err) -> M,
        M: Into<String>;

    /// Converts this type into an [`RpcResult`] with the
    /// [`jsonrpsee::types::error::INTERNAL_ERROR_CODE`] and given message and data.
    fn map_internal_err_with_data<'a, F, M>(self, op: F) -> RpcResult<Ok>
    where
        F: FnOnce(Err) -> (M, &'a [u8]),
        M: Into<String>;

    /// Adds a message to the error variant and returns an internal Error.
    ///
    /// This is shorthand for `Self::map_internal_err(|err| format!("{msg}: {err}"))`.
    fn with_message(self, msg: &str) -> RpcResult<Ok>;
}

/// An extension to used to apply error conversions to various result types
pub(crate) trait ToRpcResultExt {
    /// The `Ok` variant of the [RpcResult]
    type Ok;

    /// Maps the `Ok` variant of this type into [Self::Ok] and maps the `Err` variant into rpc
    /// error.
    fn map_ok_or_rpc_err(self) -> RpcResult<<Self as ToRpcResultExt>::Ok>;
}

/// Constructs an invalid params JSON-RPC error.
pub(crate) fn invalid_params_rpc_err(
    msg: impl Into<String>,
) -> jsonrpsee::types::error::ErrorObject<'static> {
    rpc_err(jsonrpsee::types::error::INVALID_PARAMS_CODE, msg, None)
}

/// Constructs an internal JSON-RPC error.
pub(crate) fn internal_rpc_err(
    msg: impl Into<String>,
) -> jsonrpsee::types::error::ErrorObject<'static> {
    rpc_err(jsonrpsee::types::error::INTERNAL_ERROR_CODE, msg, None)
}

/// Constructs an internal JSON-RPC error with code and message
pub(crate) fn rpc_error_with_code(
    code: i32,
    msg: impl Into<String>,
) -> jsonrpsee::types::error::ErrorObject<'static> {
    rpc_err(code, msg, None)
}

/// Constructs a JSON-RPC error, consisting of `code`, `message` and optional `data`.
pub(crate) fn rpc_err(
    code: i32,
    msg: impl Into<String>,
    data: Option<&[u8]>,
) -> jsonrpsee::types::error::ErrorObject<'static> {
    jsonrpsee::types::error::ErrorObject::owned(
        code,
        msg.into(),
        data.map(|data| {
            jsonrpsee::core::to_json_raw_value(&format!("0x{}", hex::encode(data)))
                .expect("serializing String does fail")
        }),
    )
}
