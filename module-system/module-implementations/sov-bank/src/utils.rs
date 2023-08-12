use sov_modules_api::digest::Digest;

use crate::genesis::DEPLOYER;

/// Derives token address from `token_name`, `sender` and `salt`.
pub fn get_token_address<C: sov_modules_api::Context>(
    token_name: &str,
    sender: &[u8],
    salt: u64,
) -> C::Address {
    let mut hasher = C::Hasher::new();
    hasher.update(sender);
    hasher.update(token_name.as_bytes());
    hasher.update(salt.to_le_bytes());

    let hash: [u8; 32] = hasher.finalize().into();
    C::Address::from(hash)
}

/// Gets the token address for the genesis block using the `DEPLOYER` address as the sender.
pub fn get_genesis_token_address<C: sov_modules_api::Context>(
    token_name: &str,
    salt: u64,
) -> C::Address {
    get_token_address::<C>(token_name, &DEPLOYER, salt)
}
