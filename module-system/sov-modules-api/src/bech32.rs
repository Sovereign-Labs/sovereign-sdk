use core::str::FromStr;

use bech32::{Error, FromBase32, ToBase32};
use sov_rollup_interface::maybestd::string::{String, ToString};
use sov_rollup_interface::maybestd::vec::Vec;

use crate::Address;

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

#[derive(
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    Debug,
    PartialEq,
    Clone,
    Eq,
)]
#[cfg_attr(
    all(feature = "arbitrary", feature = "std"),
    derive(arbitrary::Arbitrary, proptest_derive::Arbitrary)
)]
#[cfg_attr(
    feature = "std",
    serde(try_from = "String", into = "String"),
    derive(derive_more::Display, derive_more::Into),
    display(fmt = "{}", "value")
)]
pub struct AddressBech32 {
    value: String,
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for AddressBech32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <AddressBech32 as core::fmt::Debug>::fmt(self, f)
    }
}

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

#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Bech32ParseError {
    #[cfg_attr(feature = "std", error("Bech32 error: {0}"))]
    Bech32(#[cfg_attr(feature = "std", from)] bech32::Error),
    #[cfg_attr(feature = "std", error("Wrong HRP: {0}"))]
    WrongHPR(String),
}

#[cfg(not(feature = "std"))]
impl From<bech32::Error> for Bech32ParseError {
    fn from(e: bech32::Error) -> Self {
        Bech32ParseError::Bech32(e)
    }
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for Bech32ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        <Bech32ParseError as core::fmt::Debug>::fmt(self, f)
    }
}

#[cfg(not(feature = "std"))]
impl From<Bech32ParseError> for anyhow::Error {
    fn from(e: Bech32ParseError) -> Self {
        anyhow::Error::msg(e)
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
