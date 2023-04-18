use std::{str::FromStr, fmt};
use bech32::{ToBase32, FromBase32, Error};
use derive_more::{Into, Display};
use crate::Address;

pub fn vec_to_bech32(vec: &[u8], hrp: &str) -> Result<String, Error> {        
    let data = vec.to_base32();
    let bech32_addr = bech32::encode(hrp, data, bech32::Variant::Bech32)?;
    Ok(bech32_addr.to_string())
}

pub fn bech32_to_vec(bech32_addr: &str) -> Result<(String, Vec<u8>), Error> {
    let (hrp, data, _) = bech32::decode(bech32_addr)?;
    let vec = Vec::<u8>::from_base32(&data)?;
    Ok((hrp, vec))
}

const HRP: &str = "sov";

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, serde::Serialize, serde::Deserialize, Debug, PartialEq, Clone, Eq, Into, Display)]
#[serde(try_from = "String")]
#[serde(into = "String")]
#[display(fmt = "{}", "value")]
pub struct AddressBech32 {    
    value: String
}

impl TryFrom<&[u8]> for AddressBech32 {
    type Error = bech32::Error;

    fn try_from(addr: &[u8]) -> Result<Self, bech32::Error> {
        if addr.len() != 32 {
            return Err(bech32::Error::InvalidLength);
        }
        let string = vec_to_bech32(addr, HRP)?;
        Ok(AddressBech32{ value: string })
    }    
}

impl From<&Address> for AddressBech32 {
    fn from(addr: &Address) -> Self {
        let string = vec_to_bech32(&addr.addr, HRP).unwrap();
        AddressBech32{ value: string }
    }    
}

impl TryFrom<String> for AddressBech32 {
    type Error = Bech32ParseError;

    fn try_from(addr: String) -> Result<Self, Bech32ParseError> {
        AddressBech32::from_str(&addr)
    }    
}

#[derive(Debug)]
pub enum Bech32ParseError {   
    Bech32(bech32::Error),
    WrongHPR(String),
}

impl From<bech32::Error> for Bech32ParseError {
    fn from(err: bech32::Error) -> Self {
        Bech32ParseError::Bech32(err)
    }
}

impl fmt::Display for Bech32ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Bech32ParseError::Bech32(err) => write!(f, "Bech32 error: {}", err),
            Bech32ParseError::WrongHPR(hrp) => write!(f, "Wrong HRP: {}", hrp),
        }
    }
}

impl FromStr for AddressBech32 {
    type Err = Bech32ParseError;

    fn from_str(s: &str) -> Result<Self, Bech32ParseError> {
        let (hrp, _) = bech32_to_vec(s)?;

        if HRP != hrp {            
            return Err(Bech32ParseError::WrongHPR(hrp))
        }

        Ok(AddressBech32 {
            value: s.to_string(),
        })        
    }
}