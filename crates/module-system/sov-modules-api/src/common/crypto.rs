//! Asymmetric cryptography primitive definitions. These structures extend the traits defined in [`sov_rollup_interface::crypto`]
//! to provide more constraints that are useful for the module system.

use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::derive::Debug;
use digest::consts::U32;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sov_rollup_interface::crypto::CredentialId;
use sov_rollup_interface::crypto::SigVerificationError;
#[cfg(feature = "native")]
use sov_rollup_interface::MaybeArbitrary;
use sov_rollup_interface::{crypto::PublicKey, sov_universal_wallet::UniversalWallet};

use crate::transaction::PubKeyAndSignature;
use crate::Spec;

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

/// A wrapper around an inner [`PublicKey`] that forces particular serialization formats.
///
/// This is needed particularly for the [`crate::transaction::Transaction`] struct's `pub_key` field so that serde
/// serialization is predictable instead of relying on the serialization of the generic type,
/// which could be anything. If the type serializes to anything then it makes it impossible to know
/// how to construct a transaction in client-side libraries like the web3 SDK.
///
/// We attempted to add support for this purely using attributes on the
/// [`crate::transaction::Transaction`] struct but ran into limitations in
/// [`sov_universal_wallet`]'s `as_ty` field attribute. It does not support generics and thus there
/// is no way to know the length of a public key (it could be 32 or 33 bytes with our
/// currently supported key types). Adding support for this seems tricky so we decided on this
/// wrapper approach.
///
/// Because of the need to know the length of the public key, we use this wrapper type that borsh
/// serializes as a `Vec<u8>` instead of a slice. Thus the length of the public key is prefixed in
/// the serialization.
///
/// Additionally, this type also serde serializes as a hex string which provides a predictable and
/// readable output.
#[derive(PartialEq, Eq, Hash, Clone, Debug, JsonSchema, UniversalWallet, PartialOrd, Ord)]
pub struct VarLengthPublicKey<T: PublicKey>(pub T);

impl<T: PublicKey> PublicKey for VarLengthPublicKey<T> {
    fn credential_id(&self) -> CredentialId {
        self.0.credential_id()
    }
}

impl<T: PublicKey> TryFrom<Vec<u8>> for VarLengthPublicKey<T> {
    type Error = T::Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        T::try_from(value).map(Self)
    }
}

impl<T: PublicKey> AsRef<[u8]> for VarLengthPublicKey<T> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<T: PublicKey> AsRef<T> for VarLengthPublicKey<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T: PublicKey> std::ops::Deref for VarLengthPublicKey<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: PublicKey> std::ops::DerefMut for VarLengthPublicKey<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: PublicKey + PartialEq> PartialEq<T> for VarLengthPublicKey<T> {
    fn eq(&self, other: &T) -> bool {
        self.0 == *other
    }
}

impl<T: PublicKey> BorshSerialize for VarLengthPublicKey<T> {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let buf = self.0.as_ref().to_vec();
        BorshSerialize::serialize(&buf, writer)
    }
}

impl<T: PublicKey> BorshDeserialize for VarLengthPublicKey<T> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let buf = Vec::deserialize_reader(reader)?;
        let inner = T::try_from(buf)
            .map_err(|_| std::io::Error::other("failed to deserialize public key"))?;
        Ok(Self(inner))
    }
}

impl<T: PublicKey> serde::Serialize for VarLengthPublicKey<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            let s = hex::encode(self.0.as_ref());
            serializer.serialize_str(&s)
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl<'de, T: PublicKey> serde::Deserialize<'de> for VarLengthPublicKey<T>
where
    T: TryFrom<Vec<u8>>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let hex_string = <String as serde::Deserialize>::deserialize(deserializer)?;
            let bytes = hex::decode(&hex_string).map_err(serde::de::Error::custom)?;
            let inner = T::try_from(bytes).map_err(|_e| serde::de::Error::custom("invalid hex"))?;
            Ok(Self(inner))
        } else {
            let inner = T::deserialize(deserializer)?;
            Ok(Self(inner))
        }
    }
}

impl<T: PublicKey> std::str::FromStr for VarLengthPublicKey<T> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use anyhow::anyhow;
        let bytes = hex::decode(s)?;
        let inner = T::try_from(bytes).map_err(|_| anyhow!("failed to parse bytes"))?;
        Ok(Self(inner))
    }
}

/// A PrivateKey used in the Module System. This extends the [`sov_rollup_interface::crypto::PrivateKey`] trait by requiring
/// the `arbitrary` trait when the `arbitrary` feature is enabled.
#[cfg(feature = "native")]
pub trait PrivateKeyExt: sov_rollup_interface::crypto::PrivateKey + MaybeArbitrary {}

#[cfg(feature = "native")]
impl<P: sov_rollup_interface::crypto::PrivateKey + MaybeArbitrary> PrivateKeyExt for P {}

/// A simple K of N multisig implementation.
#[derive(BorshDeserialize, BorshSerialize, JsonSchema, Serialize, Deserialize, Debug, Clone)]
pub struct Multisig<Pubkey> {
    pub(crate) required_signers: u8,
    pub(crate) signers: Vec<Pubkey>,
}

impl<Pubkey: PublicKeyExt> Multisig<Pubkey> {
    /// Creates a new multisig.
    pub fn new(required_signers: u8, signers: Vec<Pubkey>) -> Self {
        Self {
            required_signers,
            signers,
        }
    }

    /// Computes the credential ID for the multisig.
    pub fn credential_id<H: Digest<OutputSize = U32>>(&self) -> CredentialId {
        let mut hasher = H::new();
        let mut sorted_signers = self.signers.clone();
        sorted_signers.sort();
        hasher.update(self.required_signers.to_le_bytes());
        hasher.update(borsh::to_vec(&sorted_signers).expect("Serialization to vec is infallible"));
        CredentialId::from_bytes(hasher.finalize().into())
    }

    /// Verifies the signature
    pub fn verify_signature<S: Spec>(
        &self,
        msg: &[u8],
        signatures: &[PubKeyAndSignature<S>],
    ) -> Result<(), SigVerificationError> {
        use sov_rollup_interface::crypto::PublicKey;
        use sov_rollup_interface::crypto::Signature;
        let mut signers: std::collections::HashSet<_> =
            signatures.iter().map(|s| s.key()).collect();
        let mut valid = 0;
        for PubKeyAndSignature {
            signature,
            pub_key: key,
        } in signatures.iter()
        {
            // If the signer was in the set, we remove it and verify the signature.
            if signers.remove(key) {
                signature.verify(key, msg)?;
                valid += 1;
            } else {
                return Err(SigVerificationError { error: format!("Signer with credential ID {} is not part of the multisig or has already signed", key.credential_id()) });
            }
        }

        if valid < self.required_signers {
            return Err(SigVerificationError {
                error: format!(
                    "Not enough valid signatures. Required: {}, Got: {}",
                    self.required_signers, valid
                ),
            });
        }
        Ok(())
    }
}
