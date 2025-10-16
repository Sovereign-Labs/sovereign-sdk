//! This module contains marker traits that are used to ensure compatibility with secp256k1 based
//! crypto, which is necessary for EIP712 signatures to work.
//! Of the stock `CryptoSpec` implementations provided by the SDK, `EvmCryptoSpec` is natively
//! compatible. But if your rollup uses a non-secp256k1 based CryptoSpec normally, the
//! `Secp256k1CryptoSpec` needs to be implemented with types implementing secp256k1 in order to
//! allow EIP712 signature verification.
//!
//! The SpecView machinery is necessary to hack around the fact that `Transaction<D, S>` contains
//! `S::CryptoSpec`-typed fields; the authenticator uses `Secp256k1View<Runtime::Spec>` rather than
//! `Runtime::Spec` as the Spec generic on transactions in order to enable deserialization of the
//! correct types.

use std::marker::PhantomData;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_address::EvmCryptoSpec;
use sov_modules_api::{higher_kinded_types::{Generic, HigherKindedHelper}, CryptoSpec, CryptoSpecExt, PublicKeyExt, SignatureExt, Spec};

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

#[derive(PartialEq, Eq, Default, Clone, Debug)]
pub struct Secp256k1ViewCrypto<C: CryptoSpec + Secp256k1CryptoSpec>(PhantomData<C>);
impl<C: CryptoSpec + Secp256k1CryptoSpec> CryptoSpec for Secp256k1ViewCrypto<C> {
    type Signature = <C as Secp256k1CryptoSpec>::Signature;
    type PublicKey = <C as Secp256k1CryptoSpec>::PublicKey;
    #[cfg(feature = "native")]
    type PrivateKey = <C as Secp256k1CryptoSpec>::PrivateKey;
    type Hasher = C::Hasher;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        // We should be able to return an all-0 public key here, to simply make the credential
        // unusable for any operations - but it's tricky to do so with a generic type (the
        // PublicKey trait doesn't implement Default)
        todo!();
    }
}

#[derive(PartialEq, Eq, Default, Clone, Debug, serde::Serialize, serde::Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Secp256k1ViewSpec<S>(PhantomData<S>);

impl<S: Spec> Spec for Secp256k1ViewSpec<S> where S::CryptoSpec: Secp256k1CryptoSpec {
    type Da = S::Da;
    type Address = S::Address;
    type Storage = S::Storage;
    type InnerZkvm = S::InnerZkvm;
    type OuterZkvm = S::OuterZkvm;
    type Gas = S::Gas;
    type CryptoSpec = Secp256k1ViewCrypto<S::CryptoSpec>;
}

impl<S: Spec> Generic for Secp256k1ViewSpec<S> where S::CryptoSpec: Secp256k1CryptoSpec, S: Generic {
    type With<M> = Secp256k1ViewSpec<S::With<M>>;
}

impl<S: Spec> HigherKindedHelper for Secp256k1ViewSpec<S> where S::CryptoSpec: Secp256k1CryptoSpec, S: HigherKindedHelper {
    type Inner = S::Inner;
}

/// EvmCryptoSpec uses secp256k1 crypto internally, therefore naturally implements
/// Secp256k1CryptoSpec.
impl Secp256k1CryptoSpec for EvmCryptoSpec {
    type Signature = <Self as CryptoSpec>::Signature;
    type PublicKey = <Self as CryptoSpec>::PublicKey;
    #[cfg(feature = "native")]
    type PrivateKey = <Self as CryptoSpec>::PrivateKey;
}
