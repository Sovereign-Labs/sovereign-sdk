use std::fmt::{Display, Formatter};
use std::str::FromStr;

use anyhow::Context;
use celestia_types::state::{AccAddress, AddressKind, AddressTrait};
// use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Clone, Eq, Serialize, Deserialize, Hash)] // TODO: , BorshDeserialize, BorshSerialize)]
pub struct CelestiaAddress(AccAddress);

impl AsRef<[u8]> for CelestiaAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.id_ref().as_ref()
    }
}

/// Decodes slice of bytes into CelestiaAddress
/// Treats it as string if it starts with HRP and the rest is valid ASCII
/// Otherwise just decodes the tendermint Id and creates address from that.
impl<'a> TryFrom<&'a [u8]> for CelestiaAddress {
    type Error = anyhow::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        let hrp = AddressKind::Account.prefix();

        if value.starts_with(hrp.as_bytes()) && value.is_ascii() {
            // safety, because we checked that it is ASCII
            let s = unsafe { std::str::from_utf8_unchecked(value) };
            s.parse().context("failed parsing celestia address")
        } else {
            let array = value.try_into().context("invalid slice length")?;
            let id = tendermint::account::Id::new(array);
            Ok(Self(AccAddress::new(id)))
        }
    }
}

impl Display for CelestiaAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for CelestiaAddress {
    type Err = <AccAddress as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
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
        let encoded =
            bech32::encode("celestia", input.to_base32(), bech32::Variant::Bech32).unwrap();
        let bytes = encoded.as_bytes();
        let address = CelestiaAddress::try_from(bytes);
        assert!(address.is_ok());
        let address = address.unwrap();
        let output = format!("{}", address);
        assert_eq!(encoded, output);
    }

    // 20 u8 -> 32 u5
    fn check_from_as_ref(input: [u8; 20]) {
        let encoded =
            bech32::encode("celestia", input.to_base32(), bech32::Variant::Bech32).unwrap();
        let address1 = CelestiaAddress::from_str(&encoded).unwrap();
        let bytes = address1.as_ref();
        let address = CelestiaAddress::try_from(bytes);
        assert!(address.is_ok());
        let address = address.unwrap();
        let output = format!("{}", address);
        assert_eq!(encoded, output);
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
            if let Ok(address) = result {
                let output = format!("{}", address);
                assert_eq!(input, output);
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
    }
}
