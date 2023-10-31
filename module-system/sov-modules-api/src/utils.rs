use jsonrpsee::types::ErrorObjectOwned;
use sov_modules_core::{Context, Spec};
use sov_rollup_interface::digest::Digest;

pub fn generate_address<C: Context>(key: &str) -> <C as Spec>::Address {
    let hash: [u8; 32] = <C as Spec>::Hasher::digest(key.as_bytes()).into();
    C::Address::from(hash)
}

pub fn to_jsonrpsee_error_object(err: impl ToString, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
