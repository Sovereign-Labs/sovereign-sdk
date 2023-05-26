pub mod call;
pub mod genesis;
#[cfg(feature = "native")]
pub mod query;

use sov_modules_api::{CallResponse, Context, Error, Genesis, Module};
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

#[derive(ModuleInfo, Clone)]
pub struct NonFungibleToken<C: Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub(crate) admin: sov_state::StateValue<C::Address>,

    #[state]
    pub(crate) owners: sov_state::StateMap<u64, C::Address>,
}

pub struct NonFungibleTokenConfig<C: Context> {
    pub admin: C::Address,
    pub owners: Vec<(u64, C::Address)>,
}
impl<C: Context> Genesis for NonFungibleToken<C> {
    type Context = C;

    type Config = NonFungibleTokenConfig<C>;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(config, working_set)?)
    }
}

impl<C: Context> Module for NonFungibleToken<C> {
    type CallMessage = call::CallMessage<C>;

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse, Error> {
        let call_result = match msg {
            call::CallMessage::Mint { id } => self.mint(id, context, working_set),
            call::CallMessage::Transfer { to, id } => self.transfer(id, to, context, working_set),
            call::CallMessage::Burn { id } => self.burn(id, context, working_set),
        };
        Ok(call_result?)
    }
}
