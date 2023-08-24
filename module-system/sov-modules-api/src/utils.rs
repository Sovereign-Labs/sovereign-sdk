pub use sov_sequencer::utils::to_jsonrpsee_error_object;

use crate::{Context, Digest, Spec};

pub fn generate_address<C: Context>(key: &str) -> <C as Spec>::Address {
    let hash: [u8; 32] = <C as Spec>::Hasher::digest(key.as_bytes()).into();
    C::Address::from(hash)
}
