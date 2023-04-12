use crate::{Account, Accounts};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_state::WorkingSet;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage<C: sov_modules_api::Context> {
    GetAccount(C::PublicKey),
}

const HRP: &str = "addr";

#[derive(Deserialize, Serialize, Debug, Eq, PartialEq)]
pub enum Response {
    AccountExists { 
        #[serde(serialize_with = "bech32_serde::serialize::<_, HRP>", deserialize_with = "bech32_serde::deserialize::<_, HRP>")]
        addr: Vec<u8>, 
        nonce: u64 
    },
    AccountEmpty,
}

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn get_account(
        &self,
        pub_key: C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Response {
        match self.accounts.get(&pub_key, working_set) {
            Some(Account { addr, nonce }) => Response::AccountExists {
                addr: addr.as_ref().to_vec(),
                nonce,
            },
            None => Response::AccountEmpty,
        }
    }
}

// TODO: Move to separate module?
mod bech32_serde {
    use bech32::{ToBase32, FromBase32, Error};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer, const HRP: &'static str>(vec: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {        
        let bech32_addr_str = vec_to_bech32(vec, HRP).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&bech32_addr_str)
    }

    pub fn deserialize<'de, D, const HRP: &'static str>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bech32_str = String::deserialize(deserializer)?;
        let (hrp, vec) = bech32_to_vec(&bech32_str).map_err(serde::de::Error::custom)?;
        if HRP != hrp {
            Err(serde::de::Error::custom(format!("Invalid HRP, expected {}, got {}", HRP, hrp)))
        } else {
            Ok(vec)
        }        
    }

    fn vec_to_bech32(vec: &[u8], hrp: &str) -> Result<String, Error> {        
        let data = vec.to_base32();
        let bech32_addr = bech32::encode(hrp, data, bech32::Variant::Bech32)?;
        Ok(bech32_addr.to_string())
    }
    
    fn bech32_to_vec(bech32_addr: &str) -> Result<(String, Vec<u8>), Error> {
        let (hrp, data, _) = bech32::decode(bech32_addr)?;
        let vec = Vec::<u8>::from_base32(&data)?;
        Ok((hrp, vec))
    }
}