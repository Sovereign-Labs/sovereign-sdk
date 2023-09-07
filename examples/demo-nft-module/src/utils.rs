use sov_modules_api::digest::Digest;

/// Derives token address from `collection_name`, `sender`
pub fn get_collection_address<C: sov_modules_api::Context>(
    collection_name: &str,
    sender: &[u8],
) -> C::Address {
    let mut hasher = C::Hasher::new();
    hasher.update(sender);
    hasher.update(collection_name.as_bytes());

    let hash: [u8; 32] = hasher.finalize().into();
    C::Address::from(hash)
}