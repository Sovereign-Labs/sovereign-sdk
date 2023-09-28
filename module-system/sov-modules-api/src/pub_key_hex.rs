use crate::default_signature::DefaultPublicKey;
use derive_more::Display;

use ed25519_dalek::{VerifyingKey as DalekPublicKey, PUBLIC_KEY_LENGTH};
use thiserror::Error;
#[derive(
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Debug,
    PartialEq,
    Clone,
    Eq,
    Display,
)]
#[serde(try_from = "String", into = "String")]
#[display(fmt = "{}", "hex")]
pub struct PublicKeyHex {
    hex: String,
}

#[derive(Error, Debug)]
pub enum HexConversionError {
    #[error("todo")]
    OddLength,
    #[error("todo")]
    InvalidHexCharacter { c: char, index: usize },
}

impl TryFrom<&str> for PublicKeyHex {
    type Error = HexConversionError;

    fn try_from(hex: &str) -> Result<Self, Self::Error> {
        Self::try_from(hex.to_owned())
    }
}

impl TryFrom<String> for PublicKeyHex {
    type Error = HexConversionError;

    fn try_from(hex: String) -> Result<Self, Self::Error> {
        if hex.len() & 1 != 0 {
            return Err(HexConversionError::OddLength);
        }

        if let Some((index, c)) = hex.chars().enumerate().find(|(_, c)| {
            !matches!(c, '0'..='9' | 'a'..='f')
            //Case::Upper => !matches!(c, '0'..='9' | 'A'..='F'),
        }) {
            return Err(HexConversionError::InvalidHexCharacter { c, index });
        }

        Ok(Self { hex })
    }
}

impl From<PublicKeyHex> for String {
    fn from(pub_key: PublicKeyHex) -> Self {
        pub_key.hex
    }
}

impl From<DefaultPublicKey> for PublicKeyHex {
    fn from(pub_key: DefaultPublicKey) -> Self {
        let hex = hex::encode(pub_key.pub_key.as_bytes());
        Self { hex }
    }
}

impl TryFrom<PublicKeyHex> for DefaultPublicKey {
    type Error = anyhow::Error;

    fn try_from(pub_key: PublicKeyHex) -> Result<Self, Self::Error> {
        let bytes = hex::decode(pub_key.hex)?;

        let bytes: [u8; PUBLIC_KEY_LENGTH] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid public key size"))?;

        let pub_key = DalekPublicKey::from_bytes(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid public key"))?;

        Ok(DefaultPublicKey { pub_key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pub_key_hex() {
        let pk_hex = PublicKeyHex::try_from("z").unwrap();
    }
}
