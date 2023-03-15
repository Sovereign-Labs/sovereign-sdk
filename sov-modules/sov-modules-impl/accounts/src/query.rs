use crate::{Account, Accounts};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::Deserialize;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage<C: sov_modules_api::Context> {
    GetAccount(C::PublicKey),
}

#[derive(Deserialize, Debug, Eq, PartialEq)]
pub enum Response {
    AccountExist { addr: [u8; 32], nonce: u64 },
    AccountEmpty,
}

impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn get_account(&self, pub_key: C::PublicKey) -> Response {
        match self.accounts.get(&pub_key) {
            Some(Account { addr, nonce }) => Response::AccountExist {
                addr: addr.addr,
                nonce,
            },
            None => Response::AccountEmpty,
        }
    }
}
