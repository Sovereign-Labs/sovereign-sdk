//! Cryptography optimized for the Rrisc0 Zkvm.
use std::hash::Hash;
#[cfg(feature = "native")]
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use ed25519_dalek::{
    Signature as DalekSignature, VerifyingKey as DalekPublicKey, PUBLIC_KEY_LENGTH,
};
use sov_rollup_interface::crypto::{PublicKeyHex, SigVerificationError};
use sov_rollup_interface::reexports::schemars::{self, JsonSchema};
use sov_rollup_interface::sov_universal_wallet::UniversalWallet;

/// Defines private key types and operations
#[cfg(feature = "native")]
pub mod private_key {

    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    use super::{Risc0PublicKey, Risc0Signature};

    /// A private key for the Risc0 signature scheme.
    /// This struct also stores the corresponding public key.
    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    pub struct Risc0PrivateKey {
        key_pair: SigningKey,
    }

    impl core::fmt::Debug for Risc0PrivateKey {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Risc0PrivateKey")
                .field("public_key", &self.key_pair.verifying_key())
                .field("private_key", &"***REDACTED***")
                .finish()
        }
    }

    impl sov_rollup_interface::crypto::PrivateKey for Risc0PrivateKey {
        type PublicKey = Risc0PublicKey;

        type Signature = Risc0Signature;

        fn generate() -> Self {
            let mut csprng = OsRng;

            Self {
                key_pair: SigningKey::generate(&mut csprng),
            }
        }

        fn pub_key(&self) -> Self::PublicKey {
            Risc0PublicKey {
                pub_key: self.key_pair.verifying_key(),
            }
        }

        fn sign(&self, msg: &[u8]) -> Self::Signature {
            Risc0Signature {
                msg_sig: self.key_pair.sign(msg),
            }
        }
    }

    impl Risc0PrivateKey {
        /// Returns the private key as a hex string.
        pub fn as_hex(&self) -> String {
            hex::encode(self.key_pair.to_bytes())
        }
    }

    #[cfg(feature = "arbitrary")]
    mod arbitrary_impls {
        use proptest::prelude::{any, BoxedStrategy};
        use proptest::strategy::Strategy;
        use rand::rngs::StdRng;
        use rand::SeedableRng;
        use sov_rollup_interface::crypto::PrivateKey;

        use super::*;

        impl<'a> arbitrary::Arbitrary<'a> for Risc0PrivateKey {
            fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
                // it is important to generate the secret deterministically from the arbitrary argument
                // so keys and signatures will be reproducible for a given seed.
                // this unlocks fuzzy replay

                // to generate the seed, we need to make sure there is at least 32 bytes left in the buffer.
                if u.len() < 32 {
                    return Err(arbitrary::Error::NotEnoughData);
                }

                let seed = <[u8; 32]>::arbitrary(u)?;
                let rng = &mut StdRng::from_seed(seed);
                let key_pair = SigningKey::generate(rng);

                Ok(Self { key_pair })
            }
        }

        impl<'a> arbitrary::Arbitrary<'a> for Risc0PublicKey {
            fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
                Risc0PrivateKey::arbitrary(u).map(|p| p.pub_key())
            }
        }

        impl<'a> arbitrary::Arbitrary<'a> for Risc0Signature {
            fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
                // the secret/public pair is lost; it is impossible to verify this signature
                // to run a verification, generate the keys+payload individually
                let payload_len = u.arbitrary_len::<u8>()?;
                let payload = u.bytes(payload_len)?;
                Risc0PrivateKey::arbitrary(u).map(|s| s.sign(payload))
            }
        }

        impl proptest::arbitrary::Arbitrary for Risc0PrivateKey {
            type Parameters = ();
            type Strategy = BoxedStrategy<Self>;

            fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
                any::<[u8; 32]>()
                    .prop_map(|seed| Self {
                        key_pair: SigningKey::generate(&mut StdRng::from_seed(seed)),
                    })
                    .boxed()
            }
        }

        impl proptest::arbitrary::Arbitrary for Risc0PublicKey {
            type Parameters = ();
            type Strategy = BoxedStrategy<Self>;

            fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
                any::<Risc0PrivateKey>()
                    .prop_map(|key| key.pub_key())
                    .boxed()
            }
        }

        impl proptest::arbitrary::Arbitrary for Risc0Signature {
            type Parameters = ();
            type Strategy = BoxedStrategy<Self>;

            fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
                any::<(Risc0PrivateKey, Vec<u8>)>()
                    .prop_map(|(key, bytes)| key.sign(&bytes))
                    .boxed()
            }
        }
    }
}

/// The public key of an ed25519 keypair. Wraps the optimized Risc0 fork of the ed25519-dalek crate.
#[derive(PartialEq, Eq, Hash, Clone, Debug, JsonSchema, UniversalWallet)]
pub struct Risc0PublicKey {
    #[schemars(
        flatten,
        with = "String",
        length(equal = "ed25519_dalek::PUBLIC_KEY_LENGTH * 2")
    )]
    #[sov_wallet(as_ty = "[u8; ed25519_dalek::PUBLIC_KEY_LENGTH]")]
    pub(crate) pub_key: DalekPublicKey,
}

impl Risc0PublicKey {
    /// Returns the bytes of the underlying public key.
    pub fn bytes(&self) -> &[u8; 32] {
        self.pub_key.as_bytes()
    }
}

impl sov_rollup_interface::crypto::PublicKey for Risc0PublicKey {
    fn credential_id(&self) -> sov_rollup_interface::crypto::CredentialId {
        // The pub key is already 32 bytes, so we don't hash it.
        let data = sov_rollup_interface::common::HexString(*self.bytes());
        sov_rollup_interface::crypto::CredentialId(data)
    }
}

impl BorshDeserialize for Risc0PublicKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buffer = [0; PUBLIC_KEY_LENGTH];
        reader.read_exact(&mut buffer)?;

        let pub_key = DalekPublicKey::from_bytes(&buffer).map_err(map_error)?;

        Ok(Self { pub_key })
    }
}

impl BorshSerialize for Risc0PublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(self.pub_key.as_bytes())
    }
}

/// An ed25519 signature. Wraps the optimized Risc0 fork of the ed25519-dalek crate.
#[derive(
    PartialEq, Eq, Debug, Clone, serde::Serialize, serde::Deserialize, JsonSchema, UniversalWallet,
)]
pub struct Risc0Signature {
    /// The inner signature.
    #[schemars(
        flatten,
        with = "String",
        length(equal = "ed25519_dalek::Signature::BYTE_SIZE * 2")
    )]
    #[sov_wallet(as_ty = "[u8; ed25519_dalek::Signature::BYTE_SIZE]")]
    pub msg_sig: DalekSignature,
}

impl BorshDeserialize for Risc0Signature {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buffer = [0; DalekSignature::BYTE_SIZE];
        reader.read_exact(&mut buffer)?;

        Ok(Self {
            msg_sig: DalekSignature::from_bytes(&buffer),
        })
    }
}

impl BorshSerialize for Risc0Signature {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(&self.msg_sig.to_bytes())
    }
}

impl TryFrom<&[u8]> for Risc0Signature {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self {
            msg_sig: DalekSignature::from_slice(value).map_err(anyhow::Error::msg)?,
        })
    }
}

impl sov_rollup_interface::crypto::Signature for Risc0Signature {
    type PublicKey = Risc0PublicKey;

    fn verify(&self, pub_key: &Self::PublicKey, msg: &[u8]) -> Result<(), SigVerificationError> {
        pub_key
            .pub_key
            .verify_strict(msg, &self.msg_sig)
            .map_err(|e| SigVerificationError {
                error: e.to_string(),
            })
    }
}

#[cfg(feature = "native")]
fn map_error(e: ed25519_dalek::SignatureError) -> std::io::Error {
    std::io::Error::other(e)
}
#[cfg(not(feature = "native"))]
fn map_error(_e: ed25519_dalek::SignatureError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, "Signature error")
}

#[cfg(feature = "native")]
impl FromStr for Risc0PublicKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let pk_hex = PublicKeyHex::try_from(s)?;
        Risc0PublicKey::try_from(&pk_hex)
    }
}

#[cfg(feature = "native")]
impl FromStr for Risc0Signature {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;

        let bytes: ed25519_dalek::ed25519::SignatureBytes = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid signature"))?;

        Ok(Risc0Signature {
            msg_sig: DalekSignature::from_bytes(&bytes),
        })
    }
}

#[cfg(test)]
#[cfg(feature = "native")]
mod tests {
    use sov_rollup_interface::crypto::PrivateKey;

    use super::*;

    #[test]
    fn test_privatekey_serde_bincode() {
        use self::private_key::Risc0PrivateKey;

        let key_pair = Risc0PrivateKey::generate();
        let serialized = bincode::serialize(&key_pair).expect("Serialization to vec is infallible");
        let output = bincode::deserialize::<Risc0PrivateKey>(&serialized)
            .expect("SigningKey is serialized correctly");

        assert_eq!(key_pair.as_hex(), output.as_hex());
    }

    #[test]
    fn test_privatekey_serde_json() {
        use self::private_key::Risc0PrivateKey;

        let key_pair = Risc0PrivateKey::generate();
        let serialized = serde_json::to_vec(&key_pair).expect("Serialization to vec is infallible");
        let output = serde_json::from_slice::<Risc0PrivateKey>(&serialized)
            .expect("Keypair is serialized correctly");

        assert_eq!(key_pair.as_hex(), output.as_hex());
    }
}

#[cfg(test)]
#[cfg(all(feature = "arbitrary", feature = "native"))]
mod proptest_tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn pub_key_json_schema_is_valid(item in any::<Risc0PublicKey>()) {
            let serialized = serde_json::to_value(item).unwrap();
            let schema = serde_json::to_value(schemars::schema_for!(Risc0PublicKey)).unwrap();

            jsonschema::validate(&schema, &serialized).unwrap();
        }

        #[test]
        fn sig_json_schema_is_valid(item in any::<Risc0Signature>()) {
            let serialized = serde_json::to_value(item).unwrap();
            let schema = serde_json::to_value(schemars::schema_for!(Risc0Signature)).unwrap();

            jsonschema::validate(&schema, &serialized).unwrap();
        }
    }
}

impl serde::Serialize for Risc0PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            serde::Serialize::serialize(&PublicKeyHex::from(self), serializer)
        } else {
            serde::Serialize::serialize(&self.pub_key, serializer)
        }
    }
}

impl<'de> serde::Deserialize<'de> for Risc0PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let pub_key_hex: PublicKeyHex = serde::Deserialize::deserialize(deserializer)?;
            Ok(Risc0PublicKey::try_from(&pub_key_hex).map_err(serde::de::Error::custom)?)
        } else {
            let pub_key: DalekPublicKey = serde::Deserialize::deserialize(deserializer)?;
            Ok(Risc0PublicKey { pub_key })
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_pub_key_json() {
        let pub_key_hex: PublicKeyHex =
            "022e229198d957bf0c0a504e7d7bcec99a1d62cccc7861ed2452676ad0323ad8"
                .try_into()
                .unwrap();

        let pub_key = Risc0PublicKey::try_from(&pub_key_hex).unwrap();
        let pub_key_str: String = serde_json::to_string(&pub_key).unwrap();

        assert_eq!(
            pub_key_str,
            r#""022e229198d957bf0c0a504e7d7bcec99a1d62cccc7861ed2452676ad0323ad8""#
        );

        let deserialized: Risc0PublicKey = serde_json::from_str(&pub_key_str).unwrap();
        assert_eq!(deserialized, pub_key);
    }
}

impl From<&Risc0PublicKey> for PublicKeyHex {
    fn from(pub_key: &Risc0PublicKey) -> Self {
        let hex = hex::encode(pub_key.pub_key.as_bytes());
        // UNWRAP: conversion to SafeString can error in only two cases: non-printable-ascii and too long.
        // A hex::encoded string should always be printable ascii, and a public key is 32 bytes =
        // 64 hex characters, well below the 128 character SafeString limit.
        Self {
            hex: hex.try_into().unwrap(),
        }
    }
}

impl TryFrom<&PublicKeyHex> for Risc0PublicKey {
    type Error = anyhow::Error;

    fn try_from(pub_key: &PublicKeyHex) -> Result<Self, Self::Error> {
        let bytes = hex::decode(&pub_key.hex)?;

        let bytes: [u8; PUBLIC_KEY_LENGTH] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid public key size"))?;

        let pub_key = DalekPublicKey::from_bytes(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid public key"))?;

        Ok(Risc0PublicKey { pub_key })
    }
}

#[cfg(test)]
#[cfg(feature = "native")]
mod hex_tests {
    use sov_rollup_interface::crypto::PrivateKey;

    use super::*;
    use crate::crypto::private_key::Risc0PrivateKey;

    #[test]
    fn test_pub_key_hex() {
        let pub_key = Risc0PrivateKey::generate().pub_key();
        let pub_key_hex = PublicKeyHex::from(&pub_key);
        let converted_pub_key = Risc0PublicKey::try_from(&pub_key_hex).unwrap();
        assert_eq!(pub_key, converted_pub_key);
    }

    #[test]
    fn test_pub_key_hex_str() {
        let key = "022e229198d957bf0c0a504e7d7bcec99a1d62cccc7861ed2452676ad0323ad8";
        let pub_key_hex_lower: PublicKeyHex = key.try_into().unwrap();
        let pub_key_hex_upper: PublicKeyHex = key.to_uppercase().try_into().unwrap();

        let pub_key_lower = Risc0PublicKey::try_from(&pub_key_hex_lower).unwrap();
        let pub_key_upper = Risc0PublicKey::try_from(&pub_key_hex_upper).unwrap();

        assert_eq!(pub_key_lower, pub_key_upper);
    }
}
