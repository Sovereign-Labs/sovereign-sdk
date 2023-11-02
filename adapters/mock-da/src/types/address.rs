use std::str::FromStr;

use sov_rollup_interface::{BasicAddress, RollupAddress};

/// Sequencer DA address used in tests.
pub const MOCK_SEQUENCER_DA_ADDRESS: [u8; 32] = [0u8; 32];

/// A mock address type used for testing. Internally, this type is standard 32 byte array.
#[derive(
    Debug, PartialEq, Clone, Eq, Copy, Hash, Default, borsh::BorshDeserialize, borsh::BorshSerialize,
)]
pub struct MockAddress {
    /// Underlying mock address.
    addr: [u8; 32],
}

impl MockAddress {
    /// Creates a new mock address containing the given bytes.
    pub const fn new(addr: [u8; 32]) -> Self {
        Self { addr }
    }
}

impl serde::Serialize for MockAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            serde::Serialize::serialize(&hex::encode(self.addr), serializer)
        } else {
            serde::Serialize::serialize(&self.addr, serializer)
        }
    }
}

impl<'de> serde::Deserialize<'de> for MockAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let hex_addr: String = serde::Deserialize::deserialize(deserializer)?;
            Ok(MockAddress::from_str(&hex_addr).map_err(serde::de::Error::custom)?)
        } else {
            let addr = <[u8; 32] as serde::Deserialize>::deserialize(deserializer)?;
            Ok(MockAddress { addr })
        }
    }
}

impl FromStr for MockAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let addr = hex::decode(s).map_err(anyhow::Error::msg)?;
        if addr.len() != 32 {
            return Err(anyhow::anyhow!("Invalid address length"));
        }

        let mut array = [0; 32];
        array.copy_from_slice(&addr);
        Ok(MockAddress { addr: array })
    }
}

impl<'a> TryFrom<&'a [u8]> for MockAddress {
    type Error = anyhow::Error;

    fn try_from(addr: &'a [u8]) -> Result<Self, Self::Error> {
        if addr.len() != 32 {
            anyhow::bail!("Address must be 32 bytes long");
        }
        let mut addr_bytes = [0u8; 32];
        addr_bytes.copy_from_slice(addr);
        Ok(Self { addr: addr_bytes })
    }
}

impl AsRef<[u8]> for MockAddress {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl From<[u8; 32]> for MockAddress {
    fn from(addr: [u8; 32]) -> Self {
        MockAddress { addr }
    }
}

impl std::fmt::Display for MockAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", hex::encode(self.addr))
    }
}

impl BasicAddress for MockAddress {}
impl RollupAddress for MockAddress {}

#[cfg(test)]
mod tests {
    use sov_rollup_interface::maybestd::string::ToString;

    use super::*;

    #[test]
    fn test_mock_address_string() {
        let addr = MockAddress { addr: [3u8; 32] };
        let s = addr.to_string();
        let recovered_addr = s.parse::<MockAddress>().unwrap();
        assert_eq!(addr, recovered_addr);
    }
}
