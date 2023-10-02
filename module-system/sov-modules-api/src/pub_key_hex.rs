use derive_more::Display;
use ed25519_dalek::{VerifyingKey as DalekPublicKey, PUBLIC_KEY_LENGTH};

/// A hexadecimal representation of a PublicKey.
use crate::default_signature::DefaultPublicKey;
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

impl TryFrom<&str> for PublicKeyHex {
    type Error = anyhow::Error;

    fn try_from(hex: &str) -> Result<Self, Self::Error> {
        Self::try_from(hex.to_owned())
    }
}

impl TryFrom<String> for PublicKeyHex {
    type Error = anyhow::Error;

    fn try_from(hex: String) -> Result<Self, Self::Error> {
        if hex.len() & 1 != 0 {
            anyhow::bail!("Bad hex conversion: odd input length")
        }

        if let Some((index, c)) = hex.chars().enumerate().find(|(_, c)| {
            !(matches!(c, '0'..='9' | 'a'..='f') || matches!(c, '0'..='9' | 'A'..='F'))
        }) {
            anyhow::bail!(
                "Bad hex conversion: wrong character `{}` at index {}",
                c,
                index
            )
        }

        Ok(Self { hex })
    }
}

impl From<PublicKeyHex> for String {
    fn from(pub_key: PublicKeyHex) -> Self {
        pub_key.hex
    }
}

impl From<&DefaultPublicKey> for PublicKeyHex {
    fn from(pub_key: &DefaultPublicKey) -> Self {
        let hex = hex::encode(pub_key.pub_key.as_bytes());
        Self { hex }
    }
}

impl TryFrom<&PublicKeyHex> for DefaultPublicKey {
    type Error = anyhow::Error;

    fn try_from(pub_key: &PublicKeyHex) -> Result<Self, Self::Error> {
        let bytes = hex::decode(&pub_key.hex)?;

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
    use crate::default_signature::private_key::DefaultPrivateKey;
    use crate::PrivateKey;

    #[test]
    fn test_pub_key_hex() {
        let pub_key = DefaultPrivateKey::generate().pub_key();
        let pub_key_hex = PublicKeyHex::try_from(&pub_key).unwrap();
        let converted_pub_key = DefaultPublicKey::try_from(&pub_key_hex).unwrap();
        assert_eq!(pub_key, converted_pub_key);
    }

    #[test]
    fn test_pub_key_hex_str() {
        let key = "022e229198d957bf0c0a504e7d7bcec99a1d62cccc7861ed2452676ad0323ad8";
        let pub_key_hex_lower: PublicKeyHex = key.try_into().unwrap();
        let pub_key_hex_upper: PublicKeyHex = key.to_uppercase().try_into().unwrap();

        let pub_key_lower = DefaultPublicKey::try_from(&pub_key_hex_lower).unwrap();
        let pub_key_upper = DefaultPublicKey::try_from(&pub_key_hex_upper).unwrap();

        assert_eq!(pub_key_lower, pub_key_upper)
    }

    #[test]
    fn test_bad_pub_key_hex_str() {
        let key = "022e229198d957Zf0c0a504e7d7bcec99a1d62cccc7861ed2452676ad0323ad8";
        let err = PublicKeyHex::try_from(key).unwrap_err();

        assert_eq!(
            err.to_string(),
            "Bad hex conversion: wrong character `Z` at index 14"
        );

        let key = "022";
        let err = PublicKeyHex::try_from(key).unwrap_err();

        assert_eq!(err.to_string(), "Bad hex conversion: odd input length")
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for PublicKeyHex {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let hex: String = hex::encode(String::arbitrary(u)?);
        Ok(PublicKeyHex::try_from(hex).unwrap())
    }
}
