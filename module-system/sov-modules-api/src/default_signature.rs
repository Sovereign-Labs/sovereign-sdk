#[cfg(feature = "native")]
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use ed25519_dalek::ed25519::signature::Signature as DalekSignatureTrait;
use ed25519_dalek::{
    PublicKey as DalekPublicKey, Signature as DalekSignature, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH,
};
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{SigVerificationError, Signature};

#[cfg(feature = "native")]
pub mod private_key {

    use ed25519_dalek::{Keypair, SignatureError, Signer};
    use rand::rngs::OsRng;
    use thiserror::Error;

    use super::{DefaultPublicKey, DefaultSignature};
    use crate::{Address, PrivateKey, PublicKey};

    #[derive(Error, Debug)]
    pub enum DefaultPrivateKeyHexDeserializationError {
        #[error("Hex deserialization error")]
        FromHexError(#[from] hex::FromHexError),
        #[error("PrivateKey deserialization error")]
        PrivateKeyError(#[from] SignatureError),
    }

    /// A private key for the default signature scheme.
    /// This struct also stores the corresponding public key.
    pub struct DefaultPrivateKey {
        key_pair: Keypair,
    }

    impl core::fmt::Debug for DefaultPrivateKey {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("DefaultPrivateKey")
                .field("public_key", &self.key_pair.public)
                .field("private_key", &"***REDACTED***")
                .finish()
        }
    }

    impl serde::Serialize for DefaultPrivateKey {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(&self.key_pair.to_bytes())
        }
    }

    impl<'de> serde::Deserialize<'de> for DefaultPrivateKey {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            use serde::de::Error;
            let bytes = <&'de [u8] as serde::Deserialize>::deserialize(deserializer)?;
            let key_pair = Keypair::from_bytes(bytes).map_err(D::Error::custom)?;
            Ok(Self { key_pair })
        }
    }

    impl TryFrom<&[u8]> for DefaultPrivateKey {
        type Error = anyhow::Error;

        fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
            let key_pair = Keypair::from_bytes(value)?;
            key_pair.secret.to_bytes();
            Ok(Self { key_pair })
        }
    }

    impl PrivateKey for DefaultPrivateKey {
        type PublicKey = DefaultPublicKey;

        type Signature = DefaultSignature;

        fn generate() -> Self {
            let mut csprng = OsRng;

            Self {
                key_pair: Keypair::generate(&mut csprng),
            }
        }

        fn pub_key(&self) -> Self::PublicKey {
            DefaultPublicKey {
                pub_key: self.key_pair.public,
            }
        }

        fn sign(&self, msg: &[u8]) -> Self::Signature {
            DefaultSignature {
                msg_sig: self.key_pair.sign(msg),
            }
        }
    }

    impl DefaultPrivateKey {
        pub fn as_hex(&self) -> String {
            hex::encode(self.key_pair.to_bytes())
        }

        pub fn from_hex(hex: &str) -> Result<Self, DefaultPrivateKeyHexDeserializationError> {
            let bytes = hex::decode(hex)?;
            Ok(Self {
                key_pair: Keypair::from_bytes(&bytes)?,
            })
        }

        pub fn default_address(&self) -> Address {
            self.pub_key().to_address::<Address>()
        }
    }
}

#[cfg_attr(feature = "native", derive(schemars::JsonSchema))]
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct DefaultPublicKey {
    #[cfg_attr(
        feature = "native",
        schemars(with = "&[u8]", length(equal = "ed25519_dalek::PUBLIC_KEY_LENGTH"))
    )]
    pub(crate) pub_key: DalekPublicKey,
}

impl Serialize for DefaultPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = self.pub_key.as_bytes();
        serializer.serialize_bytes(s)
    }
}

impl<'de> Deserialize<'de> for DefaultPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = <Vec<u8> as serde::Deserialize>::deserialize(deserializer)?;
        let dpk = DalekPublicKey::from_bytes(&bytes).or(Err(D::Error::custom(
            "Couldn't convert bytes to ed25519 public key",
        )))?;
        Ok(DefaultPublicKey { pub_key: dpk })
    }
}

impl BorshDeserialize for DefaultPublicKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buffer = [0; PUBLIC_KEY_LENGTH];
        reader.read_exact(&mut buffer)?;

        let pub_key = DalekPublicKey::from_bytes(&buffer).map_err(map_error)?;

        Ok(Self { pub_key })
    }
}

impl BorshSerialize for DefaultPublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(self.pub_key.as_bytes())
    }
}

#[cfg_attr(feature = "native", derive(schemars::JsonSchema))]
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct DefaultSignature {
    #[cfg_attr(
        feature = "native",
        schemars(with = "&[u8]", length(equal = "ed25519_dalek::Signature::BYTE_SIZE"))
    )]
    pub msg_sig: DalekSignature,
}

impl Serialize for DefaultSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = self.msg_sig.as_bytes();
        serializer.serialize_bytes(s)
    }
}

impl<'de> Deserialize<'de> for DefaultSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = <Vec<u8> as serde::Deserialize>::deserialize(deserializer)?;
        let dsig = DalekSignature::from_bytes(&bytes).or(Err(D::Error::custom(
            "Couldn't convert bytes to ed25519 signature",
        )))?;
        Ok(DefaultSignature { msg_sig: dsig })
    }
}

impl BorshDeserialize for DefaultSignature {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buffer = [0; SIGNATURE_LENGTH];
        reader.read_exact(&mut buffer)?;

        let msg_sig = DalekSignature::from_bytes(&buffer).map_err(map_error)?;

        Ok(Self { msg_sig })
    }
}

impl BorshSerialize for DefaultSignature {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(self.msg_sig.as_bytes())
    }
}

impl Signature for DefaultSignature {
    type PublicKey = DefaultPublicKey;

    fn verify(&self, pub_key: &Self::PublicKey, msg: &[u8]) -> Result<(), SigVerificationError> {
        pub_key
            .pub_key
            .verify_strict(msg, &self.msg_sig)
            .map_err(|e| SigVerificationError::BadSignature(e.to_string()))
    }
}

#[cfg(feature = "native")]
fn map_error(e: ed25519_dalek::SignatureError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e)
}
#[cfg(not(feature = "native"))]
fn map_error(_e: ed25519_dalek::SignatureError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, "Signature error")
}

#[cfg(feature = "native")]
impl FromStr for DefaultPublicKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;
        let pub_key = DalekPublicKey::from_bytes(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid public key"))?;
        Ok(DefaultPublicKey { pub_key })
    }
}

#[cfg(feature = "native")]
impl FromStr for DefaultSignature {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;
        let msg_sig =
            DalekSignature::from_bytes(&bytes).map_err(|_| anyhow::anyhow!("Invalid signature"))?;
        Ok(DefaultSignature { msg_sig })
    }
}

#[test]
#[cfg(feature = "native")]
fn test_privatekey_serde() {
    use self::private_key::DefaultPrivateKey;
    use crate::PrivateKey;

    let key_pair = DefaultPrivateKey::generate();
    let serialized = bincode::serialize(&key_pair).expect("Serialization to vec is infallible");
    let output = bincode::deserialize::<DefaultPrivateKey>(&serialized)
        .expect("Keypair is serialized correctly");

    assert_eq!(key_pair.as_hex(), output.as_hex());
}
