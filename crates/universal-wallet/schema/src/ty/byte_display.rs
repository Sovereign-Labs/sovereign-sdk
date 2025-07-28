use std::num::ParseIntError;

use bech32::{Bech32, Bech32m, Hrp};
use borsh::{BorshDeserialize, BorshSerialize};
use hex::FromHexError;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::display::FormatError;

#[derive(Debug, Error, PartialEq)]
pub enum ByteParseError {
    #[error("The input could not be decoded as bech32: {0}")]
    InvalidBech32(#[from] bech32::DecodeError),
    #[error("The input could not be decoded as base58: {0}")]
    InvalidBase58(#[from] bs58::decode::Error),
    #[error("The input {0} could not be decoded as a decimal array")]
    InvalidDecimal(String),
    #[error("The input contained elments that could not be parsed as integers: {0}")]
    InvalidNumber(#[from] ParseIntError),
    #[error("The input could not be decoded as a hex string: {0}")]
    InvalidHex(#[from] FromHexError),
    #[error("Invalid bech32 prefix. Expected {expected}, input contained prefix {actual}")]
    InvalidBech32Prefix { expected: String, actual: String },
    #[error("Invalid length: expected {expected} {encoding}-encoded bytes, but the input - once decoded - contained {actual} bytes")]
    InvalidLength {
        expected: usize,
        encoding: String,
        actual: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, BorshDeserialize, BorshSerialize)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ByteDisplay {
    #[default]
    Hex,
    Decimal,
    Bech32 {
        #[cfg_attr(feature = "serde", serde(with = "hrp_serde"))]
        #[borsh(
            serialize_with = "hrp_borsh::borsh_serialize",
            deserialize_with = "hrp_borsh::borsh_deserialize"
        )]
        prefix: Hrp,
    },
    Bech32m {
        #[cfg_attr(feature = "serde", serde(with = "hrp_serde"))]
        #[borsh(
            serialize_with = "hrp_borsh::borsh_serialize",
            deserialize_with = "hrp_borsh::borsh_deserialize"
        )]
        prefix: Hrp,
    },
    Base58,
}

mod hrp_borsh {
    use bech32::Hrp;
    use borsh::{BorshDeserialize, BorshSerialize};

    pub fn borsh_serialize<W: borsh::io::Write>(
        hrp: &Hrp,
        w: &mut W,
    ) -> Result<(), borsh::io::Error> {
        let s = hrp.as_str();
        BorshSerialize::serialize(&s, w)
    }

    pub fn borsh_deserialize<R: borsh::io::Read>(r: &mut R) -> Result<Hrp, borsh::io::Error> {
        let s: String = BorshDeserialize::deserialize_reader(r)?;
        Hrp::parse(&s).map_err(borsh::io::Error::other)
    }
}

#[cfg(feature = "serde")]
mod hrp_serde {
    use bech32::Hrp;
    use serde::de::{self, Unexpected};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(hrp: &Hrp, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = hrp.as_str();
        ser.serialize_str(s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Hrp, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <&str>::deserialize(d)?;
        Hrp::parse(s).map_err(|_| de::Error::invalid_value(Unexpected::Str(s), &"a valid HRP"))
    }
}
impl ByteDisplay {
    pub fn format(&self, input: &[u8], f: &mut impl core::fmt::Write) -> Result<(), FormatError> {
        match self {
            ByteDisplay::Hex => {
                f.write_str("0x")?;
                for byte in input {
                    write!(f, "{byte:02x}")?;
                }
            }
            ByteDisplay::Decimal => {
                write!(f, "{input:?}")?;
            }
            ByteDisplay::Bech32 { prefix } => {
                bech32::encode_to_fmt::<Bech32, _>(f, *prefix, input)?
            }
            ByteDisplay::Bech32m { prefix } => {
                bech32::encode_to_fmt::<Bech32m, _>(f, *prefix, input)?
            }
            ByteDisplay::Base58 => {
                let out = bs58::encode(input).into_string();
                f.write_str(&out)?;
            }
        }
        Ok(())
    }

    pub fn parse(&self, input: &str) -> Result<Vec<u8>, ByteParseError> {
        match self {
            ByteDisplay::Hex => {
                let input = input.trim_start_matches("0x");
                Ok(hex::decode(input)?)
            }
            ByteDisplay::Decimal => {
                let Some(inner) = input
                    .strip_prefix('[')
                    .and_then(|input| input.strip_suffix(']'))
                else {
                    return Err(ByteParseError::InvalidDecimal(input.to_string()));
                };
                let ret: Result<Vec<u8>, _> = inner
                    .split_terminator(',')
                    .map(|s| s.trim().parse())
                    .collect();
                Ok(ret?)
            }
            ByteDisplay::Bech32 { prefix } | ByteDisplay::Bech32m { prefix } => {
                let (parsed_prefix, bytes) = bech32::decode(input)?;
                if parsed_prefix != *prefix {
                    return Err(ByteParseError::InvalidBech32Prefix {
                        expected: prefix.to_string(),
                        actual: parsed_prefix.to_string(),
                    });
                }
                Ok(bytes)
            }
            ByteDisplay::Base58 => Ok(bs58::decode(input).into_vec()?),
        }
    }

    pub fn parse_const<const N: usize>(&self, input: &str) -> Result<[u8; N], ByteParseError> {
        let parsed_bytes = self.parse(input)?;
        let encoding = match self {
            ByteDisplay::Hex => "hex",
            ByteDisplay::Decimal => "decimal",
            ByteDisplay::Bech32 { .. } => "bech32",
            ByteDisplay::Bech32m { .. } => "bech32m",
            ByteDisplay::Base58 => "base58",
        }
        .to_string();
        <Vec<u8> as TryInto<[u8; N]>>::try_into(parsed_bytes).map_err(|bytes| {
            ByteParseError::InvalidLength {
                expected: N,
                encoding,
                actual: bytes.len(),
            }
        })
    }
}
#[cfg(test)]
mod byte_display_tests {
    use bech32::Hrp;

    use super::{ByteDisplay, ByteParseError};

    // These tests would be a lot more concise with either a) the paste! crate, or b) a proc macro.
    // But paste! is archived/unmaintained, and writing a proc macro is probably overkill

    macro_rules! test_display_passes {
        ($display:expr, $str:literal) => {
            let bytes = [12u8; 10];
            let mut out = String::new();
            let display = $display;

            display.format(&bytes, &mut out).unwrap();
            assert_eq!(out, $str);
        };
    }

    macro_rules! test_parse_passes {
        ($display:expr, $str:literal) => {
            let input = $str;
            let bytes = [12u8; 10];
            let display = $display;

            // vec parsing
            assert_eq!(display.parse(input).unwrap(), bytes.to_vec());
            // array parsing
            assert_eq!(display.parse_const(input).unwrap(), bytes);
        };
    }

    macro_rules! test_parse_rejects {
        ($display:expr, $str:literal, $encoding_for_err:literal) => {
            let input = $str;
            let display = $display;

            let result = display.parse_const::<11>(input);
            assert!(result.is_err());
            assert_eq!(
                result.err().unwrap(),
                ByteParseError::InvalidLength {
                    expected: 11,
                    encoding: $encoding_for_err.to_string(),
                    actual: 10
                }
            )
        };
    }

    #[test]
    fn test_hex_display() {
        test_display_passes!(ByteDisplay::Hex, "0x0c0c0c0c0c0c0c0c0c0c");
    }

    #[test]
    fn test_hex_parse_passes() {
        test_parse_passes!(ByteDisplay::Hex, "0x0c0c0c0c0c0c0c0c0c0c");
    }

    #[test]
    fn test_hex_parse_failures() {
        test_parse_rejects!(ByteDisplay::Hex, "0x0c0c0c0c0c0c0c0c0c0c", "hex");
    }

    #[test]
    fn test_decimal_display() {
        test_display_passes!(
            ByteDisplay::Decimal,
            "[12, 12, 12, 12, 12, 12, 12, 12, 12, 12]"
        );
    }

    #[test]
    fn test_decimal_parse_passes() {
        test_parse_passes!(
            ByteDisplay::Decimal,
            "[12, 12, 12, 12, 12, 12, 12, 12, 12, 12]"
        );
    }

    #[test]
    fn test_decimal_parse_failures() {
        test_parse_rejects!(
            ByteDisplay::Decimal,
            "[12, 12, 12, 12, 12, 12, 12, 12, 12, 12]",
            "decimal"
        );
    }

    #[test]
    fn test_bech32_display() {
        test_display_passes!(
            ByteDisplay::Bech32 {
                prefix: Hrp::parse("aa").unwrap()
            },
            "aa1psxqcrqvpsxqcrqv80wveu"
        );
    }

    #[test]
    fn test_bech32_parse_passes() {
        test_parse_passes!(
            ByteDisplay::Bech32 {
                prefix: Hrp::parse("aa").unwrap()
            },
            "aa1psxqcrqvpsxqcrqv80wveu"
        );
    }

    #[test]
    fn test_bech32_parse_failures() {
        test_parse_rejects!(
            ByteDisplay::Bech32 {
                prefix: Hrp::parse("aa").unwrap()
            },
            "aa1psxqcrqvpsxqcrqv80wveu",
            "bech32"
        );
    }

    #[test]
    fn test_bech32m_display() {
        test_display_passes!(
            ByteDisplay::Bech32m {
                prefix: Hrp::parse("aa").unwrap()
            },
            "aa1psxqcrqvpsxqcrqvjn7qu7"
        );
    }

    #[test]
    fn test_bech32m_parse_passes() {
        test_parse_passes!(
            ByteDisplay::Bech32m {
                prefix: Hrp::parse("aa").unwrap()
            },
            "aa1psxqcrqvpsxqcrqvjn7qu7"
        );
    }

    #[test]
    fn test_bech32m_parse_failures() {
        test_parse_rejects!(
            ByteDisplay::Bech32m {
                prefix: Hrp::parse("aa").unwrap()
            },
            "aa1psxqcrqvpsxqcrqvjn7qu7",
            "bech32m"
        );
    }

    #[test]
    fn test_base58_display() {
        test_display_passes!(ByteDisplay::Base58, "gFqoeNwi4sf1M");
    }

    #[test]
    fn test_base58_parse_passes() {
        test_parse_passes!(ByteDisplay::Base58, "gFqoeNwi4sf1M");
    }

    #[test]
    fn test_base58_parse_failures() {
        test_parse_rejects!(ByteDisplay::Base58, "gFqoeNwi4sf1M", "base58");
    }
}
