//! Defines queries exposed by the accounts module, along with the relevant types
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, CredentialId, Spec};

use crate::{Account, Accounts};

/// This is the response returned from the accounts_getAccount endpoint.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
#[serde(
    bound = "Addr: serde::Serialize + serde::de::DeserializeOwned",
    rename_all = "snake_case"
)]
pub enum Response<Addr> {
    /// The account corresponding to the given credential id exists.
    AccountExists {
        /// The address of the account,
        addr: Addr,
    },
    /// The account corresponding to the credential id does not exist.
    AccountEmpty,
}

impl<S: Spec> Accounts<S> {
    /// Get the account corresponding to the given credential id.
    pub fn get_account(
        &self,
        credential_id: CredentialId,
        state: &mut ApiStateAccessor<S>,
    ) -> Response<S::Address> {
        match self.accounts.get(&credential_id, state).unwrap_infallible() {
            Some(Account { addr }) => Response::AccountExists { addr },
            None => Response::AccountEmpty,
        }
    }
}
