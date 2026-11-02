#![deny(missing_docs)]
//! An Ed25519 [`CryptoSpec`] backed by `commonware-cryptography-curve25519`.
//!
//! Keys and credential ids are byte-compatible with the `ed25519-dalek` based specs, so addresses
//! do not change when switching. Signatures are not: every message is framed with
//! [`SIGNING_NAMESPACE`] before signing, and validation follows ZIP215 rather than dalek's
//! `verify_strict`.

use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use commonware_codec::{Copying, DecodeExt};
use commonware_cryptography_curve25519::signing;
use schemars::JsonSchema;
use sov_rollup_interface::common::HexString;
use sov_rollup_interface::crypto::{CredentialId, PublicKeyHex, SigVerificationError};
use sov_rollup_interface::zk::CryptoSpec;
use sov_universal_wallet::schema::OverrideSchema;

const PUBLIC_KEY_LENGTH: usize = 32;
const SIGNATURE_LENGTH: usize = 64;

/// Namespace passed to commonware's `sign`/`verify`; the signed bytes are
/// `varint(len) || namespace || msg`, i.e. `0x00 || msg` while this is empty.
pub const SIGNING_NAMESPACE: &[u8] = b"";

/// Ed25519 signatures with commonware verification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Copy, JsonSchema)]
pub struct CommonwareCryptoSpec;

impl CryptoSpec for CommonwareCryptoSpec {
    #[cfg(feature = "native")]
    type PrivateKey = private_key::Ed25519PrivateKey;
    type PublicKey = Ed25519PublicKey;
    type Hasher = sha2::Sha256;
    type Signature = Ed25519Signature;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        Ed25519PublicKey::from_bytes(&[
            0xf1, 0xac, 0x96, 0xb6, 0xad, 0x3c, 0xd6, 0xbd, 0xda, 0xf2, 0xc2, 0x3f, 0x08, 0x9d,
            0xe7, 0x3a, 0x68, 0x16, 0xf8, 0x92, 0xc1, 0xaf, 0x34, 0x5d, 0xf7, 0x0f, 0x9a, 0x57,
            0x3a, 0x86, 0xba, 0xcb,
        ])
    }
}

/// Private keys. Serialized as `{"key_pair": [<32-byte seed>]}`, the same layout as the
/// `ed25519-dalek` based specs, so existing key files keep working.
#[cfg(feature = "native")]
pub mod private_key {
    use commonware_codec::{Copying, DecodeExt};
    use commonware_cryptography_curve25519::signing;
    use sov_rollup_interface::crypto::PrivateKey;

    use super::{Ed25519PublicKey, Ed25519Signature, SIGNING_NAMESPACE};

    /// An Ed25519 private key.
    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    #[serde(from = "KeyFile", into = "KeyFile")]
    pub struct Ed25519PrivateKey {
        seed: [u8; 32],
        key: signing::SigningKey,
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct KeyFile {
        key_pair: [u8; 32],
    }

    impl From<Ed25519PrivateKey> for KeyFile {
        fn from(key: Ed25519PrivateKey) -> Self {
            Self { key_pair: key.seed }
        }
    }

    impl From<KeyFile> for Ed25519PrivateKey {
        fn from(file: KeyFile) -> Self {
            Self::from_seed(file.key_pair)
        }
    }

    impl Ed25519PrivateKey {
        fn from_seed(seed: [u8; 32]) -> Self {
            let key = signing::SigningKey::decode(Copying(&seed[..]))
                .expect("Any 32 bytes are a valid ed25519 seed");
            Self { seed, key }
        }

        /// Returns the private key as a hex string.
        pub fn as_hex(&self) -> String {
            hex::encode(self.seed)
        }

        /// Returns the private key as a byte array.
        pub fn as_bytes(&self) -> [u8; 32] {
            self.seed
        }
    }

    impl AsRef<[u8]> for Ed25519PrivateKey {
        fn as_ref(&self) -> &[u8] {
            &self.seed
        }
    }

    impl TryFrom<Vec<u8>> for Ed25519PrivateKey {
        type Error = anyhow::Error;

        fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
            let seed: [u8; 32] = value
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid private key size"))?;
            Ok(Self::from_seed(seed))
        }
    }

    impl core::fmt::Debug for Ed25519PrivateKey {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Ed25519PrivateKey")
                .field("public_key", &self.pub_key())
                .field("private_key", &"***REDACTED***")
                .finish()
        }
    }

    impl PrivateKey for Ed25519PrivateKey {
        type PublicKey = Ed25519PublicKey;
        type Signature = Ed25519Signature;

        fn generate() -> Self {
            let mut seed = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
            Self::from_seed(seed)
        }

        fn pub_key(&self) -> Self::PublicKey {
            Ed25519PublicKey {
                key: self.key.verifying_key(),
            }
        }

        fn sign(&self, msg: &[u8]) -> Self::Signature {
            Ed25519Signature {
                sig: self.key.sign(SIGNING_NAMESPACE, msg),
            }
        }
    }

    #[cfg(feature = "arbitrary")]
    impl<'a> arbitrary::Arbitrary<'a> for Ed25519PrivateKey {
        fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
            // Deterministic in the input so that fuzz cases replay.
            Ok(Self::from_seed(u.arbitrary()?))
        }
    }

    #[cfg(feature = "arbitrary")]
    impl<'a> arbitrary::Arbitrary<'a> for Ed25519PublicKey {
        fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
            Ed25519PrivateKey::arbitrary(u).map(|p| p.pub_key())
        }
    }

    #[cfg(feature = "arbitrary")]
    impl<'a> arbitrary::Arbitrary<'a> for Ed25519Signature {
        fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
            let payload_len = u.arbitrary_len::<u8>()?;
            let payload = u.bytes(payload_len)?;
            Ed25519PrivateKey::arbitrary(u).map(|s| s.sign(payload))
        }
    }
}

/// An Ed25519 public key. Any 32 bytes decode; invalid points fail at verification.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Hash, JsonSchema)]
pub struct Ed25519PublicKey {
    #[schemars(with = "&[u8]", length(equal = "PUBLIC_KEY_LENGTH"))]
    key: signing::VerifyingKey,
}

impl Ed25519PublicKey {
    fn from_bytes(bytes: &[u8; PUBLIC_KEY_LENGTH]) -> Self {
        Self {
            key: signing::VerifyingKey::decode(Copying(&bytes[..]))
                .expect("Any 32 bytes decode as a verifying key"),
        }
    }

    /// Returns the encoded public key.
    pub fn bytes(&self) -> &[u8; PUBLIC_KEY_LENGTH] {
        self.key
            .as_ref()
            .try_into()
            .expect("Verifying key is 32 bytes")
    }
}

impl TryFrom<Vec<u8>> for Ed25519PublicKey {
    type Error = anyhow::Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let bytes: [u8; PUBLIC_KEY_LENGTH] = value
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid public key size"))?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl AsRef<[u8]> for Ed25519PublicKey {
    fn as_ref(&self) -> &[u8] {
        self.key.as_ref()
    }
}

impl sov_rollup_interface::crypto::PublicKey for Ed25519PublicKey {
    fn credential_id(&self) -> CredentialId {
        // Same as the dalek-based specs: the 32-byte key is used directly.
        CredentialId(HexString(*self.bytes()))
    }
}

impl OverrideSchema for Ed25519PublicKey {
    type Output = [u8; PUBLIC_KEY_LENGTH];
}

impl BorshSerialize for Ed25519PublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(self.bytes())
    }
}

impl BorshDeserialize for Ed25519PublicKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut bytes = [0; PUBLIC_KEY_LENGTH];
        reader.read_exact(&mut bytes)?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl serde::Serialize for Ed25519PublicKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serde::Serialize::serialize(&PublicKeyHex::from(self), serializer)
        } else {
            serde::Serialize::serialize(self.bytes(), serializer)
        }
    }
}

impl<'de> serde::Deserialize<'de> for Ed25519PublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let pub_key_hex: PublicKeyHex = serde::Deserialize::deserialize(deserializer)?;
            Ed25519PublicKey::try_from(&pub_key_hex).map_err(serde::de::Error::custom)
        } else {
            let bytes: [u8; PUBLIC_KEY_LENGTH] = serde::Deserialize::deserialize(deserializer)?;
            Ok(Self::from_bytes(&bytes))
        }
    }
}

impl From<&Ed25519PublicKey> for PublicKeyHex {
    fn from(pub_key: &Ed25519PublicKey) -> Self {
        // UNWRAP: 64 lowercase hex characters are printable ASCII and within the SafeString limit.
        Self {
            hex: hex::encode(pub_key.bytes()).try_into().unwrap(),
        }
    }
}

impl TryFrom<&PublicKeyHex> for Ed25519PublicKey {
    type Error = anyhow::Error;

    fn try_from(pub_key: &PublicKeyHex) -> Result<Self, Self::Error> {
        Self::try_from(hex::decode(&pub_key.hex)?)
    }
}

impl FromStr for Ed25519PublicKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ed25519PublicKey::try_from(&PublicKeyHex::try_from(s)?)
    }
}

/// An Ed25519 signature over `SIGNING_NAMESPACE`-framed messages.
#[derive(PartialEq, Eq, Clone, Debug, JsonSchema)]
pub struct Ed25519Signature {
    #[schemars(with = "&[u8]", length(equal = "SIGNATURE_LENGTH"))]
    sig: signing::Signature,
}

impl Ed25519Signature {
    fn from_bytes(bytes: &[u8; SIGNATURE_LENGTH]) -> Self {
        Self {
            sig: signing::Signature::decode(Copying(&bytes[..]))
                .expect("Any 64 bytes decode as a signature"),
        }
    }
}

impl TryFrom<&[u8]> for Ed25519Signature {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let bytes: &[u8; SIGNATURE_LENGTH] = value
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid signature size"))?;
        Ok(Self::from_bytes(bytes))
    }
}

impl TryFrom<Vec<u8>> for Ed25519Signature {
    type Error = anyhow::Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::try_from(value.as_slice())
    }
}

impl AsRef<[u8]> for Ed25519Signature {
    fn as_ref(&self) -> &[u8] {
        self.sig.as_ref()
    }
}

impl OverrideSchema for Ed25519Signature {
    type Output = [u8; SIGNATURE_LENGTH];
}

impl BorshSerialize for Ed25519Signature {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(self.as_ref())
    }
}

impl BorshDeserialize for Ed25519Signature {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut bytes = [0; SIGNATURE_LENGTH];
        reader.read_exact(&mut bytes)?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl serde::Serialize for Ed25519Signature {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&HexString(self.as_ref()), serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Ed25519Signature {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let HexString(bytes): HexString<[u8; SIGNATURE_LENGTH]> =
            serde::Deserialize::deserialize(deserializer)?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl FromStr for Ed25519Signature {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let HexString(bytes) = HexString::<[u8; SIGNATURE_LENGTH]>::from_str(s)?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl sov_rollup_interface::crypto::Signature for Ed25519Signature {
    type PublicKey = Ed25519PublicKey;

    fn verify(&self, pub_key: &Self::PublicKey, msg: &[u8]) -> Result<(), SigVerificationError> {
        if pub_key.key.verify(SIGNING_NAMESPACE, msg, &self.sig) {
            Ok(())
        } else {
            Err(SigVerificationError {
                error: "Ed25519 signature verification failed".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::crypto::{PrivateKey, Signature};

    use super::private_key::Ed25519PrivateKey;
    use super::*;

    #[test]
    fn test_sign_verify_roundtrip() {
        let key = Ed25519PrivateKey::try_from(vec![0x5e; 32]).unwrap();
        let sig = key.sign(b"message");

        assert!(sig.verify(&key.pub_key(), b"message").is_ok());
    }

    #[test]
    fn test_verify_rejects_wrong_message() {
        let key = Ed25519PrivateKey::try_from(vec![0x5e; 32]).unwrap();
        let sig = key.sign(b"message");

        assert!(sig.verify(&key.pub_key(), b"other message").is_err());
    }

    #[test]
    fn test_signature_equals_plain_ed25519_over_framed_message() {
        use ed25519_dalek::Signer;

        let key = Ed25519PrivateKey::try_from(vec![0x5e; 32]).unwrap();
        let dalek_key = ed25519_dalek::SigningKey::from_bytes(&[0x5e; 32]);

        assert_eq!(
            key.sign(b"message").as_ref(),
            &dalek_key.sign(b"\x00message").to_bytes()[..],
            "External signers must reproduce this: plain Ed25519 over 0x00 || msg"
        );
    }

    #[test]
    fn test_private_key_json_matches_dalek_key_file_layout() {
        let key = Ed25519PrivateKey::try_from(vec![0x5e; 32]).unwrap();
        let json = serde_json::to_value(&key).unwrap();

        assert_eq!(json, serde_json::json!({ "key_pair": vec![0x5e_u8; 32] }));
    }

    #[test]
    fn test_pub_key_json_roundtrip() {
        let key = "022e229198d957bf0c0a504e7d7bcec99a1d62cccc7861ed2452676ad0323ad8";
        let pub_key = Ed25519PublicKey::from_str(key).unwrap();
        let json = serde_json::to_string(&pub_key).unwrap();

        assert_eq!(json, format!("\"{key}\""));
        assert_eq!(
            serde_json::from_str::<Ed25519PublicKey>(&json).unwrap(),
            pub_key
        );
    }

    #[test]
    fn test_signature_borsh_roundtrip() {
        let key = Ed25519PrivateKey::try_from(vec![0x5e; 32]).unwrap();
        let sig = key.sign(b"message");
        let bytes = borsh::to_vec(&sig).unwrap();

        assert_eq!(bytes.len(), SIGNATURE_LENGTH);
        assert_eq!(Ed25519Signature::try_from_slice(&bytes).unwrap(), sig);
    }
}
