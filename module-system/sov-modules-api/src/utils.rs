use crate::{Context, Digest, Spec};

pub fn generate_address<C: Context>(key: &str) -> <C as Spec>::Address {
    let hash: [u8; 32] = <C as Spec>::Hasher::digest(key.as_bytes()).into();
    C::Address::from(hash)
}

#[cfg(feature = "dep:jsonrpsee")]
pub fn to_jsonrpsee_error_object(
    err: impl ToString,
    message: &str,
) -> jsonrpsee::types::ErrorObjectOwned {
    jsonrpsee::types::ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
