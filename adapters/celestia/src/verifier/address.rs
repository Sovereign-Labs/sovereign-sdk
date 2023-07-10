use std::fmt::{Display, Formatter};
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::traits::AddressTrait;

const HRP: &str = "celestia";

#[derive(Debug, PartialEq, Clone, Eq, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
// Raw ASCII bytes, including HRP
// TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/469
pub struct CelestiaAddress(Vec<u8>);

impl AsRef<[u8]> for CelestiaAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<'a> TryFrom<&'a [u8]> for CelestiaAddress {
    type Error = anyhow::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        Ok(Self(value.to_vec()))
    }
}

impl From<[u8; 32]> for CelestiaAddress {
    fn from(value: [u8; 32]) -> Self {
        // TODO: This is completely broken with current implementation.
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/469
        Self(value.to_vec())
    }
}

impl Display for CelestiaAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let ascii_string = String::from_utf8_lossy(&self.0);
        write!(f, "{}", ascii_string)
    }
}

impl FromStr for CelestiaAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // This could be the way to save memory:
        let (hrp, _raw_address_u5, _variant) = bech32::decode(s)?;
        if hrp != HRP {
            anyhow::bail!("Incorrect HRP. Expected {} got {}", HRP, hrp);
        }
        let value = s.as_bytes().to_vec();
        Ok(Self(value))
    }
}

impl AddressTrait for CelestiaAddress {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_display_from_string() {
        let raw_address_str = "celestia1w7wcupk5gswj25c0khnkey5fwmlndx6t5aarmk";
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
}
