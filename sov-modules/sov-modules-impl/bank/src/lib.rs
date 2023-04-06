mod call;
mod genesis;
mod query;
mod token;
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;
pub use token::{Amount, Coins, Token};

#[derive(ModuleInfo)]
pub struct Bank<C: sov_modules_api::Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub names: sov_state::StateMap<String, C::Address>,

    #[state]
    pub tokens: sov_state::StateMap<C::Address, Token<C>>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Bank<C> {
    type Context = C;

    type CallMessage = call::CallMessage<C>;

    type QueryMessage = query::QueryMessage<C>;

    fn genesis(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<(), Error> {
        Ok(self.init_module(working_set)?)
    }

    fn call(
        &self,
        _msg: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        todo!()
    }

    #[cfg(feature = "native")]
    fn query(
        &self,
        _msg: Self::QueryMessage,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> sov_modules_api::QueryResponse {
        todo!()
    }
}
