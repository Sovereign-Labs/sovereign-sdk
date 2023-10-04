use core::fmt::{Display, Formatter};
use std::hash::Hash;
use std::str::FromStr;

use primitive_types::H256;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Eq, Hash)]
pub struct AvailAddress([u8; 32]);

impl sov_rollup_interface::BasicAddress for AvailAddress {}

impl Display for AvailAddress {
    fn fmt(&self, f: &mut Formatter) -> core::fmt::Result {
        let hash = H256(self.0);
        write!(f, "{hash}")
    }
}

impl AsRef<[u8]> for AvailAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<[u8; 32]> for AvailAddress {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl FromStr for AvailAddress {
    type Err = <H256 as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let h_256 = H256::from_str(s)?;

        Ok(Self(h_256.to_fixed_bytes()))
    }
}

impl<'a> TryFrom<&'a [u8]> for AvailAddress {
    type Error = anyhow::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        Ok(Self(<[u8; 32]>::try_from(value)?))
    }
}
