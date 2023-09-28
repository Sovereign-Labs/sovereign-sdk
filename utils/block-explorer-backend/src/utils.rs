use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

/// Copyright (c) Victor Polevoy
/// Published under the terms of the MIT License:
/// <https://github.com/vityafx/serde-aux/blob/master/LICENSE>.
pub fn deserialize_from_str<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + serde::Deserialize<'de>,
    <T as FromStr>::Err: Display,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrInt<T> {
        String(String),
        Number(T),
    }

    match StringOrInt::<T>::deserialize(deserializer)? {
        StringOrInt::String(s) => s.parse::<T>().map_err(serde::de::Error::custom),
        StringOrInt::Number(i) => Ok(i),
    }
}

/// A newtype wrapper around [`Vec<u8>`] which is serialized as a
/// 0x-prefixed hex string.
#[derive(Debug, Clone)]
pub struct HexString(pub Vec<u8>);

impl AsRef<Vec<u8>> for HexString {
    fn as_ref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl Display for HexString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl serde::Serialize for HexString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        hex::encode(&self.0).serialize(serializer)
    }
}

impl<'a> serde::Deserialize<'a> for HexString {
    fn deserialize<D>(deserializer: D) -> Result<HexString, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let string = String::deserialize(deserializer)?;
        // We ignore the 0x prefix if it exists.
        let s = string.strip_prefix("0x").unwrap_or(&string);

        hex::decode(s)
            .map_err(|e| anyhow::anyhow!("Failed to decode hex: {}", e))
            .map(HexString)
            .map_err(serde::de::Error::custom)
    }
}
