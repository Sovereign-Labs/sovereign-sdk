use core::fmt::Debug;
use core::hash::Hash;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::RollupAddress;

use crate::error::SigVerificationError;

/// Signature used in the Module System.
pub trait Signature: BorshDeserialize + BorshSerialize + Eq + Clone + Debug + Send + Sync {
    type PublicKey;

    fn verify(&self, pub_key: &Self::PublicKey, msg: &[u8]) -> Result<(), SigVerificationError>;
}

/// PublicKey used in the Module System.
pub trait PublicKey:
    BorshDeserialize
    + BorshSerialize
    + Eq
    + Hash
    + Clone
    + Debug
    + Send
    + Sync
    + Serialize
    + for<'a> Deserialize<'a>
{
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
    type PublicKey: PublicKey;
    type Signature: Signature<PublicKey = Self::PublicKey>;

    fn generate() -> Self;
    fn pub_key(&self) -> Self::PublicKey;
    fn sign(&self, msg: &[u8]) -> Self::Signature;
    fn to_address<A: RollupAddress>(&self) -> A {
        self.pub_key().to_address::<A>()
    }
}
