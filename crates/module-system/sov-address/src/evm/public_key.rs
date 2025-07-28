use alloy_primitives::keccak256;
use borsh::{BorshDeserialize, BorshSerialize};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::EncodedPoint;
use schemars::JsonSchema;
use sov_modules_api::macros::UniversalWallet;
use sov_rollup_interface::crypto::PublicKeyHex;

const PUBLIC_KEY_SIZE: usize = 33;

// Helper module for serde array support
mod serde_array {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(
        bytes: &[u8; super::PUBLIC_KEY_SIZE],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(bytes))
        } else {
            // Serialize each byte individually to match secp256k1's format
            use serde::ser::SerializeTuple;
            let mut seq = serializer.serialize_tuple(super::PUBLIC_KEY_SIZE)?;
            for byte in bytes {
                seq.serialize_element(byte)?;
            }
            seq.end()
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; super::PUBLIC_KEY_SIZE], D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
            bytes
                .try_into()
                .map_err(|_| serde::de::Error::custom("invalid length"))
        } else {
            // Deserialize as a sequence of bytes
            struct ArrayVisitor;

            impl<'de> serde::de::Visitor<'de> for ArrayVisitor {
                type Value = [u8; super::PUBLIC_KEY_SIZE];

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("a 33-byte array")
                }

                fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::SeqAccess<'de>,
                {
                    let mut arr = [0u8; super::PUBLIC_KEY_SIZE];
                    for (i, byte) in arr.iter_mut().enumerate() {
                        *byte = seq
                            .next_element()?
                            .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                    }
                    Ok(arr)
                }
            }

            deserializer.deserialize_tuple(super::PUBLIC_KEY_SIZE, ArrayVisitor)
        }
    }
}

/// The public key of a secp256k1 keypair.
#[derive(PartialEq, Eq, Clone, Debug, JsonSchema, UniversalWallet)]
pub struct EthereumPublicKey {
    #[schemars(flatten, with = "String", length(equal = "PUBLIC_KEY_SIZE * 2"))]
    #[sov_wallet(as_ty = "[u8; PUBLIC_KEY_SIZE]")]
    pub(crate) pub_key: k256::PublicKey,
}

impl EthereumPublicKey {
    /// Returns the bytes of the underlying public key.
    pub fn bytes(&self) -> [u8; PUBLIC_KEY_SIZE] {
        let encoded = EncodedPoint::from(&self.pub_key);
        let mut bytes = [0u8; PUBLIC_KEY_SIZE];
        bytes.copy_from_slice(encoded.as_bytes());
        bytes
    }
}

impl std::hash::Hash for EthereumPublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.bytes().hash(state);
    }
}

impl sov_rollup_interface::crypto::PublicKey for EthereumPublicKey {
    fn credential_id(&self) -> sov_rollup_interface::crypto::CredentialId {
        // Use the same method as EthereumAddress::from(&EthereumPublicKey)
        // Get the uncompressed public key bytes (without the prefix byte)
        let uncompressed = self.pub_key.to_encoded_point(false);
        let hash: [u8; 32] = keccak256(&uncompressed.as_bytes()[1..]).into();

        sov_rollup_interface::crypto::CredentialId(hash.into())
    }
}

impl BorshDeserialize for EthereumPublicKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buffer = [0; PUBLIC_KEY_SIZE];
        reader.read_exact(&mut buffer)?;

        let pub_key = k256::PublicKey::from_sec1_bytes(&buffer).map_err(std::io::Error::other)?;

        Ok(Self { pub_key })
    }
}

impl BorshSerialize for EthereumPublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let encoded = EncodedPoint::from(&self.pub_key);
        writer.write_all(encoded.as_bytes())
    }
}

#[cfg(feature = "native")]
impl std::str::FromStr for EthereumPublicKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let pk_hex = PublicKeyHex::try_from(s)?;
        EthereumPublicKey::try_from(&pk_hex)
    }
}

impl serde::Serialize for EthereumPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            serde::Serialize::serialize(&PublicKeyHex::from(self), serializer)
        } else {
            // For compatibility with secp256k1, serialize as a fixed-size array
            // This avoids the length prefix that serialize_bytes would add
            let bytes = self.bytes();
            serde_array::serialize(&bytes, serializer)
        }
    }
}

impl<'de> serde::Deserialize<'de> for EthereumPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let pub_key_hex: PublicKeyHex = serde::Deserialize::deserialize(deserializer)?;
            Ok(EthereumPublicKey::try_from(&pub_key_hex).map_err(serde::de::Error::custom)?)
        } else {
            // For compatibility with secp256k1, deserialize as a fixed-size array
            let bytes = serde_array::deserialize(deserializer)?;
            let pub_key =
                k256::PublicKey::from_sec1_bytes(&bytes).map_err(serde::de::Error::custom)?;
            Ok(EthereumPublicKey { pub_key })
        }
    }
}

impl From<&EthereumPublicKey> for PublicKeyHex {
    fn from(pub_key: &EthereumPublicKey) -> Self {
        let hex = hex::encode(pub_key.bytes());
        // UNWRAP: conversion to SafeString can error in only two cases: non-printable-ascii and too long.
        // A hex::encoded string should always be printable ascii, and a public key is 33 bytes =
        // 66 hex characters, well below the 128-character SafeString limit.
        Self {
            hex: hex.try_into().unwrap(),
        }
    }
}

impl TryFrom<&PublicKeyHex> for EthereumPublicKey {
    type Error = anyhow::Error;

    fn try_from(pub_key: &PublicKeyHex) -> Result<Self, Self::Error> {
        let bytes = hex::decode(&pub_key.hex)?;

        let bytes: [u8; PUBLIC_KEY_SIZE] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid public key size"))?;

        let pub_key = k256::PublicKey::from_sec1_bytes(&bytes)
            .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

        Ok(Self { pub_key })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_rollup_interface::crypto::PrivateKey;

    use super::*;
    use crate::evm::address::EthereumAddress;
    use crate::evm::private_key::EthereumPrivateKey;

    #[test]
    fn test_compat_with_secp256k1_keypair() {
        let secp = secp256k1::Secp256k1::new();
        let (secret_key, pub_key_secp256k1) =
            secp.generate_keypair(&mut secp256k1::rand::thread_rng());

        // Create our public key from `secp256k1`'s public key, using direct methods
        let serialized_direct = pub_key_secp256k1.serialize();
        let k256_public_key = k256::PublicKey::from_sec1_bytes(&serialized_direct).unwrap();
        let from_secp_pub_key_0 = EthereumPublicKey {
            pub_key: k256_public_key,
        };

        // The same, but
        let serialized_pub_key = bincode::serialize(&pub_key_secp256k1).unwrap();
        // self-check between bincode and direct method call
        assert_eq!(&serialized_direct[..], &serialized_pub_key);
        let from_secp_pub_key_1: EthereumPublicKey =
            bincode::deserialize(&serialized_pub_key).unwrap();

        // Create our public key from `secp256k1`'s secret key by deserializing it
        let serialized_sk = serde_json::to_string(&secret_key).unwrap();
        let our_private_key: EthereumPrivateKey = serde_json::from_str(&serialized_sk).unwrap();
        let from_our_sk = our_private_key.pub_key();

        assert_eq!(from_secp_pub_key_0, from_our_sk);
        assert_eq!(from_secp_pub_key_1, from_our_sk);

        assert_eq!(
            pub_key_secp256k1.to_string(),
            hex::encode(from_secp_pub_key_0.bytes())
        );
        assert_eq!(
            pub_key_secp256k1.to_string(),
            hex::encode(from_secp_pub_key_1.bytes())
        );
    }

    #[test]
    fn test_pubkey_binary() {
        // Test that we can serialize/deserialize to match secp256k1 format
        let priv_key = EthereumPrivateKey::generate();
        let pub_key = priv_key.pub_key();

        // Test bincode serialization
        let serialized = bincode::serialize(&pub_key).unwrap();
        println!("Our serialized: {:?}", hex::encode(&serialized));

        // Create a secp256k1 public key from our k256 key
        let secp_bytes = pub_key.bytes();
        let secp_pubkey = secp256k1::PublicKey::from_slice(&secp_bytes).unwrap();
        let secp_serialized = bincode::serialize(&secp_pubkey).unwrap();
        println!("Secp serialized: {:?}", hex::encode(&secp_serialized));

        // They should match
        assert_eq!(serialized, secp_serialized);
    }

    #[test]
    fn test_pub_key_json() {
        let pub_key_hex: PublicKeyHex =
            "0204e690e67bd9d8cfc9c310ad3468de11416feefcc86da6e73613b89677a61b80"
                .try_into()
                .unwrap();

        let pub_key = EthereumPublicKey::try_from(&pub_key_hex).unwrap();
        let pub_key_str: String = serde_json::to_string(&pub_key).unwrap();

        assert_eq!(
            pub_key_str,
            r#""0204e690e67bd9d8cfc9c310ad3468de11416feefcc86da6e73613b89677a61b80""#
        );

        let deserialized: EthereumPublicKey = serde_json::from_str(&pub_key_str).unwrap();
        assert_eq!(deserialized, pub_key);
    }

    #[test]
    fn test_pub_key_hex() {
        let pub_key = EthereumPrivateKey::generate().pub_key();
        let pub_key_hex = PublicKeyHex::from(&pub_key);
        let converted_pub_key = EthereumPublicKey::try_from(&pub_key_hex).unwrap();
        assert_eq!(pub_key, converted_pub_key);
    }

    #[test]
    fn test_pub_key_hex_str() {
        let key = "0204e690e67bd9d8cfc9c310ad3468de11416feefcc86da6e73613b89677a61b80";
        let pub_key_hex_lower: PublicKeyHex = key.try_into().unwrap();
        let pub_key_hex_upper: PublicKeyHex = key.to_uppercase().try_into().unwrap();

        let pub_key_lower = EthereumPublicKey::try_from(&pub_key_hex_lower).unwrap();
        let pub_key_upper = EthereumPublicKey::try_from(&pub_key_hex_upper).unwrap();

        assert_eq!(pub_key_lower, pub_key_upper);
    }

    #[test]
    fn test_key_to_addr() {
        let decoded = hex::decode("226634643938363630643866613337303631353535653933333134303737393434646131336530333964393237616433643366303762396136396430336339366122").unwrap();
        let key: EthereumPrivateKey = serde_json::from_slice(&decoded).unwrap();
        let found_addr: EthereumAddress = (&(key.pub_key())).into();
        assert_eq!(
            found_addr,
            EthereumAddress::from_str("0x71334bf1710D12c9f689cC819476fA589F08C64C").unwrap()
        );
    }

    #[test]
    fn test_ethereum_public_key_credential_id_to_address() {
        use sov_rollup_interface::crypto::PublicKey;
        // Create a known private key
        let decoded = hex::decode("226634643938363630643866613337303631353535653933333134303737393434646131336530333964393237616433643366303762396136396430336339366122").unwrap();
        let private_key: EthereumPrivateKey = serde_json::from_slice(&decoded).unwrap();

        // Get the public key
        let public_key = private_key.pub_key();

        // Get the ethereum address directly from the public key
        let eth_address: EthereumAddress = (&public_key).into();

        // Get the ethereum address from the public key turned into credential id turned into address
        let credential_id = public_key.credential_id();
        assert_eq!(
            "0x4efbae02ded675eac115583671334bf1710d12c9f689cc819476fa589f08c64c",
            credential_id.to_string()
        );
        let address_from_credential_id: EthereumAddress = credential_id.into();

        // They should be equal
        assert_eq!(eth_address, address_from_credential_id);

        // Verify against known address
        let expected_address =
            EthereumAddress::from_str("0x71334bf1710D12c9f689cC819476fA589F08C64C").unwrap();
        assert_eq!(eth_address, expected_address);
    }
}
