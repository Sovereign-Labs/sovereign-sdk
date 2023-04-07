use sov_modules_api::Hasher;

pub fn create_token_address<C: sov_modules_api::Context>(
    token_name: &str,
    sender_address: &C::Address,
    salt: u64,
) -> C::Address {
    let mut hasher = C::Hasher::new();
    hasher.update(sender_address.as_ref());
    hasher.update(token_name.as_bytes());
    hasher.update(&salt.to_le_bytes());

    let hash = hasher.finalize();
    // TODO remove unwrap
    C::Address::try_from(&hash).unwrap()
}
