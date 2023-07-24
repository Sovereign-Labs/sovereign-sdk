#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod hooks;

mod call;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(test)]
mod tests;

pub use call::{CallMessage, UPDATE_ACCOUNT_MSG};
#[cfg(feature = "native")]
pub use query::{AccountsRpcImpl, AccountsRpcServer, Response};
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

/// Initial configuration for sov-accounts module.
pub struct AccountConfig<C: sov_modules_api::Context> {
    /// Public keys to initialize the rollup.
    pub pub_keys: Vec<C::PublicKey>,
}

/// An account on the rollup.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Copy, Clone)]
pub struct Account<C: sov_modules_api::Context> {
    /// The address of the account.
    pub addr: C::Address,
    /// The current nonce value associated with the account.
    pub nonce: u64,
}

/// A module responsible for managing accounts on the rollup.
#[cfg_attr(feature = "native", derive(sov_modules_macros::ModuleCallJsonSchema))]
#[derive(ModuleInfo, Clone)]
pub struct Accounts<C: sov_modules_api::Context> {
    /// The address of the sov-accounts module.
    #[address]
    pub address: C::Address,

    /// Mapping from an account address to a corresponding public key.
    #[state]
    pub(crate) public_keys: sov_state::StateMap<C::Address, C::PublicKey>,

    /// Mapping from a public key to a corresponding account.
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
