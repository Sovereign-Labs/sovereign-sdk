//! This module contains marker traits that are used to ensure compatibility with secp256k1 based
//! crypto, which is necessary for EIP712 signatures to work.
//!
//! Of the stock `CryptoSpec` implementations provided by the SDK, `EvmCryptoSpec` is natively
//! compatible. But if your rollup uses a non-secp256k1 based CryptoSpec normally, the
//! `Secp256k1CryptoSpec` needs to be implemented with types implementing secp256k1 in order to
//! allow EIP712 signature verification.

use std::marker::PhantomData;

use sov_address::EvmCryptoSpec;
use sov_modules_api::{CryptoHelper, CryptoSpec, CryptoSpecExt, PublicKeyExt, SignatureExt};

/// Marker trait for CryptoSpec implementations which are compatible with secp256k1 signatures.
/// Note that we cannot place static bounds on the logic of the implementation here - it is up to
/// the user to ensure this trait is only implemented using types that genuinely implement
/// secp256k1-based crypto.
pub trait Secp256k1CryptoSpec: CryptoSpecExt {
    /// The type implementing secp256k1-compatible digital signatures.
    type Signature: SignatureExt<PublicKey = <Self as Secp256k1CryptoSpec>::PublicKey>;
    /// The public key used for secp256k1 digital signature verification.
    type PublicKey: PublicKeyExt;
    /// The private key used for secp256k1 digital signing.
    #[cfg(feature = "native")]
    type PrivateKey: sov_modules_api::PrivateKeyExt<
        PublicKey = <Self as Secp256k1CryptoSpec>::PublicKey,
        Signature = <Self as Secp256k1CryptoSpec>::Signature,
    >;
}

/// EvmCryptoSpec uses secp256k1 crypto internally, therefore naturally implements
/// Secp256k1CryptoSpec.
impl Secp256k1CryptoSpec for EvmCryptoSpec {
    type Signature = <Self as CryptoSpec>::Signature;
    type PublicKey = <Self as CryptoSpec>::PublicKey;
    #[cfg(feature = "native")]
    type PrivateKey = <Self as CryptoSpec>::PrivateKey;
}

/// Helper wrapper type for arbitrary cryptospecs that exposes the EvmCryptoSpec for secp256k1
/// compatible crypto.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct CryptoSpecWithSecp256k1<C>(PhantomData<C>);
impl<C: CryptoSpec> CryptoSpec for CryptoSpecWithSecp256k1<C> {
    type Hasher = C::Hasher;
    type PublicKey = C::PublicKey;
    type Signature = C::Signature;
    type PrivateKey = C::PrivateKey;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        C::sovereign_admin_pubkey()
    }
}

impl<C: CryptoSpec + CryptoSpecExt> Secp256k1CryptoSpec for CryptoSpecWithSecp256k1<C> {
    type Signature = <sov_address::EvmCryptoSpec as CryptoHelper>::ExtendedSignature;
    type PublicKey = <sov_address::EvmCryptoSpec as CryptoHelper>::ExtendedPublicKey;
    type PrivateKey = <sov_address::EvmCryptoSpec as CryptoHelper>::ExtendedPrivateKey;
}
