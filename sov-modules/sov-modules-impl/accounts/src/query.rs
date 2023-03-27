use crate::{Account, Accounts};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::Address;
use sov_state::WorkingSet;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage<C: sov_modules_api::Context> {
    GetAccount(C::PublicKey),
}

#[derive(Deserialize, Serialize, Debug, Eq, PartialEq)]
pub enum Response {
    AccountExists { addr: Vec<u8>, nonce: u64 },
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
                addr: addr.as_bytes().to_vec(),
                nonce,
            },
            None => Response::AccountEmpty,
        }
    }
}
