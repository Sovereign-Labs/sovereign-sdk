use sov_modules_api::AddressBech32;
use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;

use crate::{Account, Accounts};

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Response {
    AccountExists { addr: AddressBech32, nonce: u64 },
    AccountEmpty,
}

#[rpc_gen(client, server, namespace = "accounts")]
impl<C: sov_modules_api::Context> Accounts<C> {
    #[rpc_method(name = "getAccount")]
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
