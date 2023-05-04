#[cfg(feature = "native")]
use crate::{Account, Accounts};
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "native")]
use sov_modules_api::AddressBech32;
#[cfg(feature = "native")]
use sov_state::WorkingSet;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage<C: sov_modules_api::Context> {
    GetAccount(C::PublicKey),
}

#[cfg(feature = "native")]
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Response {
    AccountExists { addr: AddressBech32, nonce: u64 },
    AccountEmpty,
}

#[cfg(feature = "native")]
impl<C: sov_modules_api::Context> Accounts<C> {
    pub(crate) fn get_account(
        &self,
        pub_key: C::PublicKey,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Response {
        match self.accounts.get(&pub_key, working_set) {
            Some(Account { addr, nonce }) => Response::AccountExists {
                addr: addr.into(),
                nonce,
            },
            None => Response::AccountEmpty,
        }
    }
}
