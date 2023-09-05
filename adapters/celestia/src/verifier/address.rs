use std::fmt::{Display, Formatter};
use std::str::FromStr;

use bech32::WriteBase32;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Human Readable Part: "celestia" for Celestia network
const HRP: &str = "celestia";
/// Bech32 variant is used for Celestia and CosmosSDK
const VARIANT: bech32::Variant = bech32::Variant::Bech32;

/// Representation of the address in the Celestia network
/// <https://github.com/celestiaorg/celestia-specs/blob/e59efd63a2165866584833e91e1cb8a6ed8c8203/src/specs/data_structures.md#address>
/// Spec says: "Addresses have a length of 32 bytes.", but in reality it is 32 `u5` elements, which can be compressed as 20 bytes.
/// TODO: Switch to bech32::u5 when it has repr transparent: <https://github.com/Sovereign-Labs/sovereign-sdk/issues/646>
#[derive(
    Debug, PartialEq, Clone, Eq, Serialize, Deserialize, BorshDeserialize, BorshSerialize, Hash,
)]
pub struct CelestiaAddress([u8; 32]);

impl AsRef<[u8]> for CelestiaAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// Decodes slice of bytes into CelestiaAddress
/// Treats it as string if it starts with HRP and the rest is valid ASCII
/// Otherwise just checks if it contains valid `u5` elements and has the correct length.
impl<'a> TryFrom<&'a [u8]> for CelestiaAddress {
    type Error = anyhow::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        if value.starts_with(HRP.as_bytes()) && value.is_ascii() {
            // safety, because we checked that it is ASCII
            let s = unsafe { std::str::from_utf8_unchecked(value) };
            return CelestiaAddress::from_str(s).map_err(|e| anyhow::anyhow!("{}", e));
        }
        if value.len() != 32 {
            anyhow::bail!("An address must be 32 u5 long");
        }
        let mut raw_address = [0u8; 32];
        for (idx, &item) in value.iter().enumerate() {
            bech32::u5::try_from_u8(item)
                .map_err(|e| anyhow::anyhow!("Element at {} is not u5: {}", idx, e))?;
            raw_address[idx] = item;
        }
        Ok(Self(raw_address))
    }
}

impl Display for CelestiaAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut w = bech32::Bech32Writer::new(HRP, VARIANT, f)?;
        for elem in self.0.iter() {
            // It is ok to unwrap, because we always sanitize data
            w.write_u5(bech32::u5::try_from_u8(*elem).unwrap())?;
        }
        w.finalize()
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
/// An error which occurs while decoding a `CelestialAddress` from a string.
pub enum CelestiaAddressFromStrError {
    /// The address has an invalid human-readable prefix.
    /// Valid addresses must start with the prefix 'celestia'.
    #[error("The address has an invalid human-readable prefix. Valid addresses must start with the prefix 'celestia', but this one began with {0}")]
    InvalidHumanReadablePrefix(String),
    /// The address has an invalid human-readable prefix.
    /// Valid addresses must start with the prefix 'celestia'.
    #[error("The address has an invalid bech32 variant. Valid addresses must be encoded in Bech32, but this is encoded in Bech32m")]
    InvalidVariant,
    /// The address could not be decoded as valid bech32
    #[error("The address could not be decoded as valid bech32: {0}")]
    InvalidBech32(#[from] bech32::Error),
}

impl FromStr for CelestiaAddress {
    type Err = CelestiaAddressFromStrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (hrp, raw_address_u5, variant) = bech32::decode(s)?;
        if hrp != HRP {
            return Err(CelestiaAddressFromStrError::InvalidHumanReadablePrefix(hrp));
        }
        if variant != VARIANT {
            return Err(CelestiaAddressFromStrError::InvalidVariant);
        }
        if raw_address_u5.len() != 32 {
            return Err(CelestiaAddressFromStrError::InvalidBech32(
                bech32::Error::InvalidLength,
            ));
        }

        let mut value: [u8; 32] = [0; 32];

        for (idx, &item) in raw_address_u5.iter().enumerate() {
            value[idx] = item.to_u8();
        }
        Ok(Self(value))
    }
}

impl sov_rollup_interface::BasicAddress for CelestiaAddress {}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use bech32::ToBase32;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_address_display_from_string() {
        let raw_address_str = "celestia1hvp2nfz3r6nqt8mlrzqf9ctwle942tkr0wql75";
        let address = CelestiaAddress::from_str(raw_address_str).unwrap();
        let output = format!("{}", address);
        assert_eq!(raw_address_str, output);
    }

    #[test]
    fn test_address_display_try_vec() {
        let raw_address_str = "celestia1w7wcupk5gswj25c0khnkey5fwmlndx6t5aarmk";
        let raw_address: Vec<u8> = raw_address_str.bytes().collect();
        let address = CelestiaAddress::try_from(&raw_address[..]).unwrap();
        let output = format!("{}", address);
        assert_eq!(raw_address_str, output);
    }

    #[test]
    fn test_from_str_and_from_slice_same() {
        let raw_address_str = "celestia1w7wcupk5gswj25c0khnkey5fwmlndx6t5aarmk";
        let raw_address_array = *b"celestia1w7wcupk5gswj25c0khnkey5fwmlndx6t5aarmk";

        let address_from_str = CelestiaAddress::from_str(raw_address_str).unwrap();
        let address_from_slice = CelestiaAddress::try_from(&raw_address_array[..]).unwrap();

        assert_eq!(address_from_str, address_from_slice);
    }

    // 20 u8 -> 32 u5
    fn check_from_bytes_as_ascii(input: [u8; 20]) {
        let encoded = bech32::encode("celestia", input.to_base32(), VARIANT).unwrap();
        let bytes = encoded.as_bytes();
        let address = CelestiaAddress::try_from(bytes);
        assert!(address.is_ok());
        let address = address.unwrap();
        let output = format!("{}", address);
        assert_eq!(encoded, output);
    }

    // 20 u8 -> 32 u5
    fn check_from_as_ref(input: [u8; 20]) {
        let encoded = bech32::encode("celestia", input.to_base32(), VARIANT).unwrap();
        let address1 = CelestiaAddress::from_str(&encoded).unwrap();
        let bytes = address1.as_ref();
        let address = CelestiaAddress::try_from(bytes);
        assert!(address.is_ok());
        let address = address.unwrap();
        let output = format!("{}", address);
        assert_eq!(encoded, output);
    }

    // 20 u8 -> 32 u5
    fn check_borsh(input: [u8; 20]) {
        let address_str = bech32::encode("celestia", input.to_base32(), VARIANT).unwrap();

        let address = CelestiaAddress::from_str(&address_str).unwrap();
        let serialized = BorshSerialize::try_to_vec(&address).unwrap();
        let deserialized = CelestiaAddress::try_from_slice(&serialized).unwrap();

        assert_eq!(deserialized, address);

        let address_str2 = format!("{}", deserialized);
        assert_eq!(address_str2, address_str);
    }

    proptest! {
        #[test]
        fn test_try_from_any_slice(input in prop::collection::vec(any::<u8>(), 0..100)) {
            let _ = black_box(CelestiaAddress::try_from(&input[..]));
        }

        #[test]
        fn test_from_str_anything(input in "\\PC*") {
            let _ = black_box(CelestiaAddress::from_str(&input));
        }

        #[test]
        // According to spec, alphanumeric characters excluding "1" "b" "i" and "o"
        fn test_from_str_lowercase_ascii(input in "celestia1[023456789ac-hj-np-z]{38}") {
            let result = CelestiaAddress::from_str(&input);
            match result {
                Ok(address) => {
                    let output = format!("{}", address);
                    assert_eq!(input, output);
                }
                Err(err) => {
                    assert_eq!(CelestiaAddressFromStrError::InvalidBech32(bech32::Error::InvalidChecksum), err);
                },
            }
        }

        #[test]
        fn test_try_from_ascii_slice(input in proptest::array::uniform20(0u8..=255)) {
            check_from_bytes_as_ascii(input);
        }

        #[test]
        fn test_try_as_ref_from(input in proptest::array::uniform20(0u8..=255)) {
            check_from_as_ref(input);
        }

        #[test]
        fn test_borsh(input in proptest::array::uniform20(0u8..=255)) {
            check_borsh(input);
        }
    }
}
