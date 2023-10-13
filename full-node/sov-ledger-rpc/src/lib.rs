//! JSON-RPC server and client implementations for Sovereign SDK rollups.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

/// A 32-byte hash [`serde`]-encoded as a hex string optionally prefixed with
/// `0x`.
#[derive(Debug, Copy, Clone, serde::Serialize, serde::Deserialize)]
pub struct HexHash(#[serde(with = "rpc_hex")] pub [u8; 32]);

mod rpc_hex {
    use core::fmt;
    use std::marker::PhantomData;

    use hex::{FromHex, ToHex};
    use serde::de::{Error, Visitor};
    use serde::{Deserializer, Serializer};

    /// Serializes `data` as hex string using lowercase characters and prefixing with '0x'.
    ///
    /// Lowercase characters are used (e.g. `f9b4ca`). The resulting string's length
    /// is always even, each byte in data is always encoded using two hex digits.
    /// Thus, the resulting string contains exactly twice as many bytes as the input
    /// data.
    pub fn serialize<S, T>(data: T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: ToHex,
    {
        let formatted_string = format!("0x{}", data.encode_hex::<String>());
        serializer.serialize_str(&formatted_string)
    }

    /// Deserializes a hex string into raw bytes.
    ///
    /// Both, upper and lower case characters are valid in the input string and can
    /// even be mixed (e.g. `f9b4ca`, `F9B4CA` and `f9B4Ca` are all valid strings).
    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromHex,
        <T as FromHex>::Error: fmt::Display,
    {
        struct HexStrVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for HexStrVisitor<T>
        where
            T: FromHex,
            <T as FromHex>::Error: fmt::Display,
        {
            type Value = T;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a hex encoded string")
            }

            fn visit_str<E>(self, data: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let data = data.trim_start_matches("0x");
                FromHex::from_hex(data).map_err(Error::custom)
            }

            fn visit_borrowed_str<E>(self, data: &'de str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let data = data.trim_start_matches("0x");
                FromHex::from_hex(data).map_err(Error::custom)
            }
        }

        deserializer.deserialize_str(HexStrVisitor(PhantomData))
    }
}
