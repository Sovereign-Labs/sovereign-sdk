use std::fmt::{Debug, Display};
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::ser::SerializeSeq;

use super::SafeVec;
use crate as sov_rollup_interface;
use crate::da::BlockHashTrait;
use crate::sov_universal_wallet::UniversalWallet; // Needed for UniversalWallet, as it requires global paths

/// A [`hex`]-encoded 32-byte hash. Note, this is not necessarily a transaction
/// hash, rather a generic hash.
pub type HexHash = HexString<[u8; 32]>;

/// A [`serde`]-compatible newtype wrapper around [`Vec<u8>`] or other
/// bytes-like types, which is serialized as a 0x-prefixed hex string.
#[derive(
    Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::AsRef, UniversalWallet,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
pub struct HexString<T = Vec<u8>>(pub T)
where
    T: AsRef<[u8]>;

impl schemars::JsonSchema for HexString {
    fn schema_name() -> String {
        "HexString".to_string()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": "^0x[a-fA-F0-9]+$",
            "description": "A `0x`-prefixed hexadecimal string (uppercase or lowercase) of variable length.",
        }))
        .unwrap()
    }
}

// Useful for representing Ethereum addresses
impl<const N: usize> schemars::JsonSchema for HexString<[u8; N]> {
    fn schema_name() -> String {
        "HexHash".to_string()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": format!("^0x[a-fA-F0-9]{{{}}}$", N * 2),
            "description": format!("{} bytes in hexadecimal format, with `0x` prefix.", N),
        }))
        .unwrap()
    }
}

impl<const N: usize> schemars::JsonSchema for HexString<SafeVec<u8, N>> {
    fn schema_name() -> String {
        "HexHash".to_string()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": format!("^0x[a-fA-F0-9]{{0,{}}}$", N * 2),
            "description": format!("At most {} bytes in hexadecimal format, with `0x` prefix.", N),
        }))
        .unwrap()
    }
}

impl<T: AsRef<[u8]>> HexString<T> {
    /// Creates a new [`HexString`] from its inner contents.
    pub const fn new(bytes: T) -> Self {
        Self(bytes)
    }
}

impl<T: AsRef<[u8]>> AsRef<[u8]> for HexString<T> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<HexHash> for [u8; 32] {
    fn from(hash: HexHash) -> Self {
        hash.0
    }
}

impl<T: AsRef<[u8]>> From<T> for HexString<T> {
    fn from(bytes: T) -> Self {
        Self(bytes)
    }
}

impl<T> Display for HexString<T>
where
    T: AsRef<[u8]>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl<T> Debug for HexString<T>
where
    T: AsRef<[u8]>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl<T> serde::Serialize for HexString<T>
where
    T: AsRef<[u8]>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_string())
        } else {
            let inner_ref = self.0.as_ref();
            let mut seq = serializer.serialize_seq(Some(inner_ref.len()))?;
            for element in inner_ref {
                seq.serialize_element(element)?;
            }
            seq.end()
        }
    }
}

impl<'de, T> serde::Deserialize<'de> for HexString<T>
where
    T: TryFrom<Vec<u8>> + AsRef<[u8]>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = if deserializer.is_human_readable() {
            let string: String = serde::Deserialize::deserialize(deserializer)?;
            parse_vec_u8(&string).map_err(serde::de::Error::custom)?
        } else {
            serde::Deserialize::deserialize(deserializer)?
        };

        Ok(HexString(bytes.try_into().map_err(|_| {
            serde::de::Error::custom("Invalid hex string length")
        })?))
    }
}

impl<T: BorshSerialize + AsRef<[u8]>> BorshSerialize for HexString<T> {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.0.serialize(writer)
    }
}

impl<T: BorshDeserialize + AsRef<[u8]>> BorshDeserialize for HexString<T> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        T::deserialize_reader(reader).map(Self)
    }
}

impl<T: TryFrom<Vec<u8>> + AsRef<[u8]>> FromStr for HexString<T> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = parse_vec_u8(s)?;
        Ok(HexString(bytes.try_into().map_err(|_| {
            anyhow::anyhow!("Invalid hex string length")
        })?))
    }
}

impl BlockHashTrait for HexHash {}

impl From<digest::generic_array::GenericArray<u8, digest::typenum::U32>> for HexHash {
    fn from(value: digest::generic_array::GenericArray<u8, digest::typenum::U32>) -> Self {
        HexHash::new(value.into())
    }
}

/// [`serde`] (de)serialization functions for [`HexString`], to be used with
/// `#[serde(with = "...")]`.
pub mod hex_string_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::HexString;

    /// Serializes `data` as hex string using lowercase characters and prefixing with '0x'.
    ///
    /// Lowercase characters are used (e.g. `f9b4ca`). The resulting string's length
    /// is always even, each byte in data is always encoded using two hex digits.
    /// Thus, the resulting string contains exactly twice as many bytes as the input
    /// data.
    pub fn serialize<S, T>(data: T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: AsRef<[u8]>,
    {
        HexString::<T>::new(data).serialize(serializer)
    }

    /// Deserializes a hex string into raw bytes.
    ///
    /// Both upper and lower case characters are valid in the input string and can
    /// even be mixed.
    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: TryFrom<Vec<u8>> + AsRef<[u8]>,
    {
        HexString::<T>::deserialize(deserializer).map(|s| s.0)
    }
}

fn parse_vec_u8(s: &str) -> anyhow::Result<Vec<u8>> {
    let s = s
        .strip_prefix("0x")
        .ok_or_else(|| anyhow::anyhow!("Missing 0x prefix"))?;

    hex::decode(s).map_err(|e| anyhow::anyhow!(e))
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::str::FromStr;

    use super::*;

    /// Serializes, then deserializes a value with [`serde_json`], then asserts
    /// equality.
    fn test_serialization_roundtrip_equality_json<T>(item: T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
    {
        let serialized = serde_json::to_string(&item).unwrap();
        let deserialized: T = serde_json::from_str(&serialized).unwrap();
        assert_eq!(item, deserialized);
    }

    fn test_serialization_roundtrip_equality_binary<T>(item: T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
    {
        let serialized = bincode::serialize(&item).unwrap();
        let deserialized: T = bincode::deserialize(&serialized).unwrap();
        assert_eq!(item, deserialized);
    }

    fn test_str_roundtrip(item: HexString) {
        let s = item.to_string();
        let restored = HexString::from_str(&s).expect("HexString::from_str should pass");
        assert_eq!(item, restored);
    }

    #[test_strategy::proptest]
    fn hex_string_serialization_roundtrip(item: HexString) {
        test_serialization_roundtrip_equality_json(item.clone());
        test_serialization_roundtrip_equality_binary(item);
    }

    #[test_strategy::proptest]
    fn hex_string_str_roundtrip(item: HexString) {
        test_str_roundtrip(item);
    }
}
