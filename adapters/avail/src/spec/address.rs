use core::fmt::{Display, Formatter};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::traits::AddressTrait;
use subxt::utils::H256;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Eq)]
pub struct AvailAddress(pub [u8; 32]);

impl AddressTrait for AvailAddress {}

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

impl<'a> TryFrom<&'a [u8]> for AvailAddress {
    type Error = anyhow::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        Ok(Self(<[u8; 32]>::try_from(value)?))
    }
}
