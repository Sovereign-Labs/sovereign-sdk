pub mod app;
#[cfg(test)]
mod data_generation;

#[cfg(feature = "native")]
pub mod helpers;
pub mod runtime;
#[cfg(test)]
mod tests;
mod tx_hooks_impl;
#[cfg(test)]
mod tx_revert_tests;
mod tx_verifier_impl;

#[cfg(feature = "native")]
use sov_modules_api::{
    default_context::DefaultContext,
    default_signature::{private_key::DefaultPrivateKey, DefaultSignature},
    Hasher, Spec,
};

pub use tx_verifier_impl::Transaction;

#[cfg(feature = "native")]
pub fn sign_tx(priv_key: &DefaultPrivateKey, message: &[u8], nonce: u64) -> DefaultSignature {
    let mut hasher = <DefaultContext as Spec>::Hasher::new();
    hasher.update(message);
    hasher.update(&nonce.to_le_bytes());
    let msg_hash = hasher.finalize();
    priv_key.sign(msg_hash)
}
