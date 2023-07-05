use std::str::FromStr;

use bech32::{Error, FromBase32, ToBase32};
use derive_more::{Display, Into};

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
    Into,
    Display,
)]
#[serde(try_from = "String", into = "String")]
#[display(fmt = "{}", "value")]
pub struct AddressBech32 {
    value: String,
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

#[derive(Debug, thiserror::Error)]
pub enum Bech32ParseError {
    #[error("Bech32 error: {0}")]
    Bech32(#[from] bech32::Error),
    #[error("Wrong HRP: {0}")]
    WrongHPR(String),
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
