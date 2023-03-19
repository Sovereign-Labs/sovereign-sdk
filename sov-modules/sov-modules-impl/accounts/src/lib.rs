pub mod hooks;

mod call;
mod genesis;
mod query;
#[cfg(test)]
mod tests;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::{Address, Error};
use sov_modules_macros::ModuleInfo;

#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq, Copy, Clone)]
pub struct Account {
    pub addr: Address,
    pub nonce: u64,
}

#[derive(ModuleInfo)]
pub struct Accounts<C: sov_modules_api::Context> {
    #[state]
    pub(crate) public_keys: sov_state::StateMap<Address, C::PublicKey, C::Storage>,

    #[state]
    pub(crate) accounts: sov_state::StateMap<C::PublicKey, Account, C::Storage>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Accounts<C> {
    type Context = C;

    type CallMessage = call::CallMessage<C>;

    type QueryMessage = query::QueryMessage<C>;

    fn genesis(&mut self) -> Result<(), Error> {
        Ok(self.init_module()?)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Self::Context,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::UpdatePublicKey(new_pub_key, sig) => {
                Ok(self.update_public_key(new_pub_key, sig, context)?)
            }
        }
    }

    #[cfg(feature = "native")]
    fn query(&self, msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
        match msg {
            query::QueryMessage::GetAccount(pub_key) => {
                let response = serde_json::to_vec(&self.get_account(pub_key)).unwrap();
                sov_modules_api::QueryResponse { response }
            }
        }
    }
}
