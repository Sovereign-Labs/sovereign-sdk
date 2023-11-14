//! Asymmetric cryptography primitive definitions

use core::fmt::Debug;
use core::hash::Hash;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::RollupAddress;

use crate::common::SigVerificationError;

/// Signature used in the Module System.
pub trait Signature:
    BorshDeserialize
    + BorshSerialize
    + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
    + Eq
    + Clone
    + Debug
    + Send
    + Sync
{
    /// The public key associated with the key pair of the signature.
    type PublicKey;

    /// Verifies the signature.
    fn verify(&self, pub_key: &Self::PublicKey, msg: &[u8]) -> Result<(), SigVerificationError>;
}

/// PublicKey used in the Module System.
pub trait PublicKey:
    BorshDeserialize
    + BorshSerialize
    + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
    + Eq
    + Hash
    + Clone
    + Debug
    + Send
    + Sync
    + Serialize
    + for<'a> Deserialize<'a>
{
    /// Returns a representation of the public key that can be represented as a rollup address.
    fn to_address<A: RollupAddress>(&self) -> A;
}

/// A PrivateKey used in the Module System.
#[cfg(feature = "native")]
pub trait PrivateKey:
    Debug
    + Send
    + Sync
    + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
    + Serialize
    + serde::de::DeserializeOwned
{
    /// The public key associated with the key pair.
    type PublicKey: PublicKey;

    /// The signature associated with the key pair.
    type Signature: Signature<PublicKey = Self::PublicKey>;

    /// Generates a new key pair, using a static entropy.
    fn generate() -> Self;

    /// Returns the public key associated with this private key.
    fn pub_key(&self) -> Self::PublicKey;

    /// Sign the provided message.
    fn sign(&self, msg: &[u8]) -> Self::Signature;

    /// Returns a representation of the public key that can be represented as a rollup address.
    fn to_address<A: RollupAddress>(&self) -> A {
        self.pub_key().to_address::<A>()
    }
}
