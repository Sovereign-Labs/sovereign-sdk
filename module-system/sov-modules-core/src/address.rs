use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use core::str::FromStr;

use bech32::{Error, FromBase32, ToBase32};
use borsh::{BorshDeserialize, BorshSerialize};
use derive_more::{Display, Into};
use sov_rollup_interface::{BasicAddress, RollupAddress};

use crate::error::Bech32ParseError;

#[derive(
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Debug,
    PartialEq,
    Clone,
    Eq,
    Into,
    Display,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
#[serde(try_from = "String", into = "String")]
#[display(fmt = "{}", "value")]
pub struct AddressBech32 {
    value: String,
}

#[cfg_attr(all(feature = "native", feature = "std"), derive(schemars::JsonSchema))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(PartialEq, Clone, Copy, Eq, BorshDeserialize, BorshSerialize, Hash)]
pub struct Address {
    addr: [u8; 32],
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl Address {
    /// Creates a new address containing the given bytes
    pub const fn new(addr: [u8; 32]) -> Self {
        Self { addr }
    }
}

impl<'a> TryFrom<&'a [u8]> for Address {
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

impl FromStr for Address {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AddressBech32::from_str(s)
            .map_err(|e| anyhow::anyhow!(e))
            .map(|addr_bech32| addr_bech32.into())
    }
}

impl From<[u8; 32]> for Address {
    fn from(addr: [u8; 32]) -> Self {
        Self { addr }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", AddressBech32::from(self))
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", AddressBech32::from(self))
    }
}

impl From<AddressBech32> for Address {
    fn from(addr: AddressBech32) -> Self {
        Self {
            addr: addr.to_byte_array(),
        }
    }
}

impl serde::Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            serde::Serialize::serialize(&AddressBech32::from(self), serializer)
        } else {
            serde::Serialize::serialize(&self.addr, serializer)
        }
    }
}

impl<'de> serde::Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let address_bech32: AddressBech32 = serde::Deserialize::deserialize(deserializer)?;
            Ok(Address::from(address_bech32.to_byte_array()))
        } else {
            let addr = <[u8; 32] as serde::Deserialize>::deserialize(deserializer)?;
            Ok(Address { addr })
        }
    }
}

impl BasicAddress for Address {}
impl RollupAddress for Address {}

pub fn vec_to_bech32m(vec: &[u8], hrp: &str) -> Result<String, Error> {
    let data = vec.to_base32();
    let bech32_addr = bech32::encode(hrp, data, bech32::Variant::Bech32m)?;
    Ok(bech32_addr)
}

pub fn bech32m_to_decoded_vec(bech32_addr: &str) -> Result<(String, Vec<u8>), Error> {
    let (hrp, data, _) = bech32::decode(bech32_addr)?;
    let vec = Vec::<u8>::from_base32(&data)?;
    Ok((hrp, vec))
}

const HRP: &str = "sov";

impl AddressBech32 {
    pub(crate) fn to_byte_array(&self) -> [u8; 32] {
        let (_, data) = bech32m_to_decoded_vec(&self.value).unwrap();

        if data.len() != 32 {
            panic!("Invalid length {}, should be 32", data.len())
        }

        let mut addr_bytes = [0u8; 32];
        addr_bytes.copy_from_slice(&data);

        addr_bytes
    }
}

impl TryFrom<&[u8]> for AddressBech32 {
    type Error = bech32::Error;

    fn try_from(addr: &[u8]) -> Result<Self, bech32::Error> {
        if addr.len() != 32 {
            return Err(bech32::Error::InvalidLength);
        }
        let string = vec_to_bech32m(addr, HRP)?;
        Ok(AddressBech32 { value: string })
    }
}

impl From<&Address> for AddressBech32 {
    fn from(addr: &Address) -> Self {
        let string = vec_to_bech32m(&addr.addr, HRP).unwrap();
        AddressBech32 { value: string }
    }
}

impl From<Address> for AddressBech32 {
    fn from(addr: Address) -> Self {
        let string = vec_to_bech32m(&addr.addr, HRP).unwrap();
        AddressBech32 { value: string }
    }
}

impl TryFrom<String> for AddressBech32 {
    type Error = Bech32ParseError;

    fn try_from(addr: String) -> Result<Self, Bech32ParseError> {
        AddressBech32::from_str(&addr)
    }
}

impl FromStr for AddressBech32 {
    type Err = Bech32ParseError;

    fn from_str(s: &str) -> Result<Self, Bech32ParseError> {
        let (hrp, _) = bech32m_to_decoded_vec(s)?;

        if HRP != hrp {
            return Err(Bech32ParseError::WrongHPR(hrp));
        }

        Ok(AddressBech32 {
            value: s.to_string(),
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_address_serialization() {
        let address = Address::from([11; 32]);
        let data: String = serde_json::to_string(&address).unwrap();
        let deserialized_address = serde_json::from_str::<Address>(&data).unwrap();

        assert_eq!(address, deserialized_address);
        assert_eq!(
            deserialized_address.to_string(),
            "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9stup8tx"
        );
    }
}
