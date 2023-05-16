pub mod hooks;

mod call;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(test)]
mod tests;

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

/// Initial configuration for Accounts module.
pub struct AccountConfig<C: sov_modules_api::Context> {
    pub pub_keys: Vec<C::PublicKey>,
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Copy, Clone)]
pub struct Account<C: sov_modules_api::Context> {
    pub addr: C::Address,
    pub nonce: u64,
}

#[derive(ModuleInfo, Clone)]
pub struct Accounts<C: sov_modules_api::Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub(crate) public_keys: sov_state::StateMap<C::Address, C::PublicKey>,

    #[state]
    pub(crate) accounts: sov_state::StateMap<C::PublicKey, Account<C>>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Accounts<C> {
    type Context = C;

    type Config = AccountConfig<C>;

    type CallMessage = call::CallMessage<C>;

    #[cfg(feature = "native")]
    type QueryMessage = query::QueryMessage<C>;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            call::CallMessage::UpdatePublicKey(new_pub_key, sig) => {
                Ok(self.update_public_key(new_pub_key, sig, context, working_set)?)
            }
        }
    }

    #[cfg(feature = "native")]
    fn query(
        &self,
        msg: Self::QueryMessage,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> sov_modules_api::QueryResponse {
        match msg {
            query::QueryMessage::GetAccount(pub_key) => {
                let response = serde_json::to_vec(&self.get_account(pub_key, working_set)).unwrap();
                sov_modules_api::QueryResponse { response }
            }
        }
    }
}
