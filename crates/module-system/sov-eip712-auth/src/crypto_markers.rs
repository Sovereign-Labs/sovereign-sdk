//! This module contains marker traits that are used to ensure compatibility with secp256k1 based
//! crypto, which is necessary for EIP712 signatures to work.
//!
//! Of the stock `CryptoSpec` implementations provided by the SDK, `EvmCryptoSpec` is natively
//! compatible. But if your rollup uses a non-secp256k1 based CryptoSpec normally, the
//! `Secp256k1CryptoSpec` needs to be implemented with types implementing secp256k1 in order to
//! allow EIP712 signature verification.

use std::marker::PhantomData;

use sov_address::EvmCryptoSpec;
use sov_modules_api::{CryptoSpec, CryptoSpecExt};

/// Marker trait for CryptoSpec implementations which are compatible with secp256k1 signatures.
/// Note that we cannot place static bounds on the logic of the implementation here - it is up to
/// the user to ensure this trait is only implemented using types that genuinely implement
/// secp256k1-based crypto.
pub trait Secp256k1CryptoSpec: CryptoSpecExt {
    /// The CryptoSpec implementing secp256k1-compatible cryptography.
    type CryptoSpec: CryptoSpecExt;
}

/// EvmCryptoSpec uses secp256k1 crypto internally, therefore naturally implements
/// Secp256k1CryptoSpec.
impl Secp256k1CryptoSpec for EvmCryptoSpec {
    type CryptoSpec = Self;
}

/// Helper wrapper type for arbitrary cryptospecs that exposes the EvmCryptoSpec for secp256k1
/// compatible crypto.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct CryptoSpecWithSecp256k1<C>(PhantomData<C>);
impl<C: CryptoSpec> CryptoSpec for CryptoSpecWithSecp256k1<C> {
    type Hasher = C::Hasher;
    type PublicKey = C::PublicKey;
    type Signature = C::Signature;
    #[cfg(feature = "native")]
    type PrivateKey = C::PrivateKey;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        C::sovereign_admin_pubkey()
    }
}

impl<C: CryptoSpec + CryptoSpecExt> Secp256k1CryptoSpec for CryptoSpecWithSecp256k1<C> {
    type CryptoSpec = sov_address::EvmCryptoSpec;
}
