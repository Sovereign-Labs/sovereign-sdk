//! Asymmetric cryptography primitive definitions. These structures extend the traits defined in [`sov_rollup_interface::crypto`]
//! to provide more constraints that are useful for the module system.

use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
#[cfg(feature = "native")]
use sov_rollup_interface::MaybeArbitrary;

/// An extended digital signature.
/// This extends the [`sov_rollup_interface::crypto::Signature`] trait by requiring [`JsonSchema`] and
/// that the signature can be serialized and deserialized using Borsh. Borsh serialization and deserialization
/// is used to serialize and deserialize standard rollup transactions.
#[cfg(not(feature = "native"))]
pub trait SignatureExt:
    sov_rollup_interface::crypto::Signature + BorshDeserialize + BorshSerialize + JsonSchema
{
}

#[cfg(not(feature = "native"))]
impl<
        S: sov_rollup_interface::crypto::Signature + BorshDeserialize + BorshSerialize + JsonSchema,
    > SignatureExt for S
{
}

/// An extended digital signature.
/// This extends the [`sov_rollup_interface::crypto::Signature`] trait by requiring [`JsonSchema`] and
/// that the signature can be serialized and deserialized using Borsh. Borsh serialization and deserialization
/// is used to serialize and deserialize standard rollup transactions.
/// When the `native` feature is enabled, we also have access to the [`std::str::FromStr`] trait.
#[cfg(feature = "native")]
pub trait SignatureExt:
    sov_rollup_interface::crypto::Signature
    + BorshDeserialize
    + BorshSerialize
    + JsonSchema
    + std::str::FromStr<Err = anyhow::Error>
{
}

#[cfg(feature = "native")]
impl<
        S: sov_rollup_interface::crypto::Signature
            + BorshDeserialize
            + BorshSerialize
            + std::str::FromStr<Err = anyhow::Error>
            + JsonSchema,
    > SignatureExt for S
{
}

/// PublicKey used in the Module System. This extends the [`sov_rollup_interface::crypto::PublicKey`] trait by requiring
/// [`JsonSchema`] and that the public key can be serialized and deserialized using Borsh. Borsh serialization and deserialization
/// is used to serialize and deserialize standard rollup transactions.
/// When the `native` feature is enabled, we also have access to the [`std::str::FromStr`] trait.
#[cfg(feature = "native")]
pub trait PublicKeyExt:
    sov_rollup_interface::crypto::PublicKey
    + BorshDeserialize
    + BorshSerialize
    + JsonSchema
    + std::str::FromStr<Err = anyhow::Error>
{
}

#[cfg(feature = "native")]
impl<
        P: sov_rollup_interface::crypto::PublicKey
            + BorshDeserialize
            + BorshSerialize
            + JsonSchema
            + std::str::FromStr<Err = anyhow::Error>,
    > PublicKeyExt for P
{
}

/// Public key used in the Module System. This extends the [`sov_rollup_interface::crypto::PublicKey`] trait by requiring
/// [`JsonSchema`] and that the public key can be serialized and deserialized using Borsh. Borsh serialization and deserialization
/// is used to serialize and deserialize standard rollup transactions.
#[cfg(not(feature = "native"))]
pub trait PublicKeyExt:
    sov_rollup_interface::crypto::PublicKey + BorshDeserialize + BorshSerialize + JsonSchema
{
}

#[cfg(not(feature = "native"))]
impl<
        P: sov_rollup_interface::crypto::PublicKey + BorshDeserialize + BorshSerialize + JsonSchema,
    > PublicKeyExt for P
{
}

/// A PrivateKey used in the Module System. This extends the [`sov_rollup_interface::crypto::PrivateKey`] trait by requiring
/// the `arbitrary` trait when the `arbitrary` feature is enabled.
#[cfg(feature = "native")]
pub trait PrivateKeyExt: sov_rollup_interface::crypto::PrivateKey + MaybeArbitrary {}

#[cfg(feature = "native")]
impl<P: sov_rollup_interface::crypto::PrivateKey + MaybeArbitrary> PrivateKeyExt for P {}
