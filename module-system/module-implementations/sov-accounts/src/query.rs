//! Defines rpc queries exposed by the accounts module, along with the relevant types
use jsonrpsee::core::RpcResult;
use sov_modules_api::macros::rpc_gen;
use sov_modules_api::{AddressBech32, WorkingSet};

use crate::{Account, Accounts};

/// This is the response returned from the accounts_getAccount endpoint.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub enum Response {
    /// The account corresponding to the given public key exists.
    AccountExists {
        /// The address of the account,
        addr: AddressBech32,
        /// The nonce of the account.
        nonce: u64,
    },
    /// The account corresponding to the given public key does not exist.
    AccountEmpty,
}

#[rpc_gen(client, server, namespace = "accounts")]
impl<C: sov_modules_api::Context> Accounts<C> {
    #[rpc_method(name = "getAccount")]
    /// Get the account corresponding to the given public key.
    pub fn get_account(
        &self,
        pub_key: C::PublicKey,
        working_set: &mut WorkingSet<C>,
    ) -> RpcResult<Response> {
        let response = match self.accounts.get(&pub_key, working_set) {
            Some(Account { addr, nonce }) => Response::AccountExists {
                addr: addr.into(),
                nonce,
            },
            None => Response::AccountEmpty,
        };

        Ok(response)
    }
}
