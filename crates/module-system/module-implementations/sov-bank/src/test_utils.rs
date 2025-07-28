use sov_modules_api::digest::Digest;
use sov_modules_api::{CryptoSpec, Spec};

use crate::TokenId;

impl TokenId {
    /// Generates a deterministic token id by hashing the input string
    pub fn generate<S: Spec>(seed: &str) -> Self {
        let hash: [u8; 32] = <S::CryptoSpec as CryptoSpec>::Hasher::digest(seed.as_bytes()).into();
        hash.into()
    }
}
