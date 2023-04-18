use crate::{Account, Accounts};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_state::WorkingSet;
use sov_modules_api::AddressBech32;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage<C: sov_modules_api::Context> {
    GetAccount(C::PublicKey),
}

#[derive(Deserialize, Serialize, Debug, Eq, PartialEq)]
pub enum Response {
    AccountExists {         
        addr: AddressBech32, 
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
                addr: AddressBech32::try_from(addr.as_ref()).unwrap(),
                nonce,
            },
            None => Response::AccountEmpty,
        }
    }
}