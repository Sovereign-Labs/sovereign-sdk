pub mod hooks;

mod call;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(test)]
mod tests;

pub use call::CallMessage;
pub use query::{AccountsRpcImpl, AccountsRpcServer, Response};
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

/// Initial configuration for sov-bank module.
pub struct AccountConfig<C: sov_modules_api::Context> {
    pub pub_keys: Vec<C::PublicKey>,
}

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Copy, Clone)]
pub struct Account<C: sov_modules_api::Context> {
    pub addr: C::Address,
    pub nonce: u64,
}

#[cfg_attr(feature = "native", derive(sov_modules_macros::ModuleCallJsonSchema))]
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
}
