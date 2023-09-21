#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod call;
pub use call::CallMessage;
mod genesis;
#[cfg(feature = "native")]
mod query;
#[cfg(feature = "native")]
pub use query::*;
use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo, WorkingSet};

#[derive(ModuleInfo, Clone)]
/// Module for non-fungible tokens (NFT).
/// Each token is represented by a unique ID.
pub struct NonFungibleToken<C: Context> {
    #[address]
    /// The address of the NonFungibleToken module.
    address: C::Address,

    #[state]
    /// Admin of the NonFungibleToken module.
    admin: sov_modules_api::StateValue<C::Address>,

    #[state]
    /// Mapping of tokens to their owners
    owners: sov_modules_api::StateMap<u64, C::Address>,
}

/// Config for the NonFungibleToken module.
/// Sets admin and existing owners.
pub struct NonFungibleTokenConfig<C: Context> {
    /// Admin of the NonFungibleToken module.
    pub admin: C::Address,
    /// Existing owners of the NonFungibleToken module.
    pub owners: Vec<(u64, C::Address)>,
}

impl<C: Context> Module for NonFungibleToken<C> {
    type Context = C;

    type Config = NonFungibleTokenConfig<C>;

    type CallMessage = CallMessage<C>;

    fn genesis(&self, config: &Self::Config, working_set: &mut WorkingSet<C>) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C>,
    ) -> Result<CallResponse, Error> {
        let call_result = match msg {
            CallMessage::Mint { id } => self.mint(id, context, working_set),
            CallMessage::Transfer { to, id } => self.transfer(id, to, context, working_set),
            CallMessage::Burn { id } => self.burn(id, context, working_set),
        };
        Ok(call_result?)
    }
}
