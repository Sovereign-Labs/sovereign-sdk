use std::fmt::Display;
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use thiserror::Error;

use crate::schema::{IndexLinking, Item, Link, Primitive, Schema, UniversalWallet};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SchemaStringError {
    #[error("String was too long: {length}, maximum: {max}")]
    StringTooLong { length: usize, max: usize },
    #[error("String contained invalid character: {character}. Only printable ASCII characters are allowed.")]
    InvalidCharacter { character: char },
}

/// A String wrapper which enforces certain constraints to ensure it is safely displayable as part
/// of a transaction without confusing the user. Only printable ASCII is allowed, and the length is
/// limited.
///
/// `UniversalWallet` implementation is forbidden on `std::String` by default, to avoid the possibility
/// of untrusted input supplying highly confusing text that tricks users into misunderstanding the
/// transaction they are signing. `SafeString` enforces some constraints to mitigate this risk. If
/// you need to encode a large data blob such as a hex string, use a `Vec<u8>` with the
/// `[sov_wallet(display = "hex")]` attribute (or any of the other display styles). Avoid raw
/// `String`s if possible.
/// If an actual `String` is absolutely necessary, then a newtype wrapper can be used, on which
/// `UniversalWallet` is derived manually.
#[derive(
    Default, Hash, Clone, PartialEq, Eq, PartialOrd, Ord, BorshSerialize, BorshDeserialize,
)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)
)]
#[cfg_attr(feature = "serde", serde(try_from = "String", into = "String"))]
pub struct SizedSafeString<const MAX_LEN: usize>(String);

pub const DEFAULT_MAX_STRING_LENGTH: usize = 128;
pub type SafeString = SizedSafeString<DEFAULT_MAX_STRING_LENGTH>;

impl<const MAX_LEN: usize> SizedSafeString<MAX_LEN> {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A convenience method to get the maximum length of SafeString instance
    pub const fn max_len(&self) -> usize {
        MAX_LEN
    }

    /// Return an empty SafeString. This method does not allocate
    pub const fn new() -> Self {
        Self(String::new())
    }

    /// Returns the length (*not* capacity or max_length) of the string in bytes
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the string is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Appends the given `char`` to the end of this `SizedSafeString` if possible.
    pub fn try_push(&mut self, c: char) -> Result<(), SchemaStringError> {
        if self.len() >= MAX_LEN {
            return Err(SchemaStringError::StringTooLong {
                length: self.len() + 1,
                max: MAX_LEN,
            });
        }

        if !Self::is_valid_char(c) {
            return Err(SchemaStringError::InvalidCharacter { character: c });
        }
        self.0.push(c);
        Ok(())
    }

    /// Returns true if the character is a valid member of `SizedSafeString`
    pub const fn is_valid_char(c: char) -> bool {
        c.is_ascii() && !c.is_ascii_control()
    }
}

impl<const MAX_LEN: usize> TryFrom<String> for SizedSafeString<MAX_LEN> {
    type Error = SchemaStringError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > MAX_LEN {
            return Err(SchemaStringError::StringTooLong {
                length: value.len(),
                max: MAX_LEN,
            });
        }
        if let Some(invalid_c) = value.chars().find(|c| !Self::is_valid_char(*c)) {
            return Err(SchemaStringError::InvalidCharacter {
                character: invalid_c,
            });
        }
        Ok(Self(value))
    }
}

impl<const MAX_LEN: usize> FromStr for SizedSafeString<MAX_LEN> {
    type Err = SchemaStringError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.try_into()
    }
}

impl<const MAX_LEN: usize> UniversalWallet for SizedSafeString<MAX_LEN> {
    fn scaffold() -> Item<IndexLinking> {
        Item::Atom(Primitive::String)
    }
    fn get_child_links(_schema: &mut Schema) -> Vec<Link> {
        Vec::new()
    }
}

impl<'a, const MAX_LEN: usize> TryFrom<&'a str> for SizedSafeString<MAX_LEN> {
    type Error = SchemaStringError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        value.to_string().try_into()
    }
}

impl<const MAX_LEN: usize> From<SizedSafeString<MAX_LEN>> for String {
    fn from(value: SizedSafeString<MAX_LEN>) -> Self {
        value.0
    }
}

impl<const MAX_LEN: usize> AsRef<[u8]> for SizedSafeString<MAX_LEN> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<const MAX_LEN: usize> AsRef<str> for SizedSafeString<MAX_LEN> {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl<const MAX_LEN: usize> std::fmt::Debug for SizedSafeString<MAX_LEN> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<const MAX_LEN: usize> Display for SizedSafeString<MAX_LEN> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{SafeString, SchemaStringError, SizedSafeString};

    #[test]
    fn test_sizedsafestring_maxlen() {
        let string_good: String = ['a'; 31].iter().collect();
        let string_bad: String = ['a'; 32].iter().collect();

        let conversion_good = <SizedSafeString<31>>::try_from(string_good);
        assert!(conversion_good.is_ok());

        let conversion_bad = <SizedSafeString<31>>::try_from(string_bad);
        assert_eq!(
            conversion_bad,
            Err(SchemaStringError::StringTooLong {
                length: 32,
                max: 31
            })
        );
    }

    #[test]
    fn test_safestring_default_len() {
        let string_good: String = ['a'; 128].iter().collect();
        let string_bad: String = ['a'; 129].iter().collect();

        let conversion_good = SafeString::try_from(string_good);
        assert!(conversion_good.is_ok());

        let conversion_bad = SafeString::try_from(string_bad);
        assert_eq!(
            conversion_bad,
            Err(SchemaStringError::StringTooLong {
                length: 129,
                max: 128
            })
        );
    }

    #[test]
    fn test_safestring_rejects_nonascii() {
        let string = "hello •";
        let conversion = SafeString::try_from(string);
        assert_eq!(
            conversion,
            Err(SchemaStringError::InvalidCharacter { character: '•' })
        );
    }

    #[test]
    fn test_safestring_rejects_control_chars() {
        let string = "hello \n world";
        let conversion = SafeString::try_from(string);
        assert_eq!(
            conversion,
            Err(SchemaStringError::InvalidCharacter { character: '\n' })
        );
    }

    #[test]
    fn json_deserializing_safestring_accepts_valid() {
        let de: SafeString = serde_json::from_str("\"Good string\"").unwrap();
        let expected: SafeString = "Good string".try_into().unwrap();
        assert_eq!(de, expected);
    }

    #[test]
    fn json_deserializing_safestring_rejects_invalid() {
        let de: Result<SafeString, _> = serde_json::from_str("\"Bad•string\"");
        assert!(de.is_err());
        assert_eq!(
            de.unwrap_err().to_string(),
            "String contained invalid character: •. Only printable ASCII characters are allowed."
        );
    }
}
