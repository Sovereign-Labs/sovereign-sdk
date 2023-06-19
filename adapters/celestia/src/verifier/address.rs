use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::traits::AddressTrait;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[derive(Debug, PartialEq, Clone, Eq, Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
pub struct CelestiaAddress(pub Vec<u8>);

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
        Self(value.to_vec())
    }
}

impl Display for CelestiaAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl FromStr for CelestiaAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Remove the "0x" prefix, if it exists.
        let s = s.strip_prefix("0x").unwrap_or(s);
        let bytes = hex::decode(s)?;
        Ok(CelestiaAddress(bytes))
    }
}

impl AddressTrait for CelestiaAddress {}
