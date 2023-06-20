use crate::{SigVerificationError, Signature};
use borsh::{BorshDeserialize, BorshSerialize};
use ed25519_dalek::{
    ed25519::signature::Signature as DalekSignatureTrait, PublicKey as DalekPublicKey,
    Signature as DalekSignature,
};
use ed25519_dalek::{PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use std::str::FromStr;

#[cfg(feature = "native")]
pub mod private_key {

    use crate::{Address, PublicKey};

    use super::{DefaultPublicKey, DefaultSignature};
    use ed25519_dalek::{Keypair, SignatureError, Signer};
    use rand::rngs::OsRng;
    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum DefaultPrivateKeyHexDeserializationError {
        #[error("Hex deserialization error")]
        FromHexError(#[from] hex::FromHexError),
        #[error("PrivateKey deserialization error")]
        PrivateKeyError(#[from] SignatureError),
    }

    pub struct DefaultPrivateKey {
        key_pair: Keypair,
    }

    impl DefaultPrivateKey {
        pub fn generate() -> Self {
            let mut csprng = OsRng;

            Self {
                key_pair: Keypair::generate(&mut csprng),
            }
        }

        pub fn sign(&self, msg: [u8; 32]) -> DefaultSignature {
            DefaultSignature {
                msg_sig: self.key_pair.sign(&msg),
            }
        }

        pub fn pub_key(&self) -> DefaultPublicKey {
            DefaultPublicKey {
                pub_key: self.key_pair.public,
            }
        }

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

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct DefaultPublicKey {
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

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct DefaultSignature {
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

    fn verify(
        &self,
        pub_key: &Self::PublicKey,
        msg_hash: [u8; 32],
    ) -> Result<(), SigVerificationError> {
        pub_key
            .pub_key
            .verify_strict(&msg_hash, &self.msg_sig)
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


impl FromStr for DefaultPublicKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;
        let pub_key = DalekPublicKey::from_bytes(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid public key"))?;
        Ok(DefaultPublicKey { pub_key })
    }
}

impl FromStr for DefaultSignature {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s)?;
        let msg_sig = DalekSignature::from_bytes(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid signature"))?;
        Ok(DefaultSignature { msg_sig })
    }
}
