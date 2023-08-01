use jsonrpsee::types::ErrorObjectOwned;

pub fn to_jsonrpsee_error_object(err: anyhow::Error, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
