use sov_modules_api::Hasher;

// Const used for creating `special_address`
const SPECIAL: [u8; 32] = [0; 32];

/// Derives token address from `token_name`, `sender` and `salt`.
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
    C::Address::from(hash)
}

/// Derives `special address` for a given token.
pub fn create_special_address<C: sov_modules_api::Context>(
    token_address: &C::Address,
) -> C::Address {
    let mut hasher = C::Hasher::new();
    hasher.update(token_address.as_ref());
    hasher.update(&SPECIAL);

    let hash = hasher.finalize();
    C::Address::from(hash)
}
