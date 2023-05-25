use crate::{Account, Accounts};
use sov_modules_api::AddressBech32;
use sov_state::WorkingSet;

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Response {
    AccountExists { addr: AddressBech32, nonce: u64 },
    AccountEmpty,
}

impl<C: sov_modules_api::Context> Accounts<C> {
    pub fn get_account(
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
