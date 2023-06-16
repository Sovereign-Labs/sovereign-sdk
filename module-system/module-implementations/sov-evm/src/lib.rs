use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

pub mod call;
mod evm;
pub mod genesis;
#[cfg(feature = "native")]
pub mod query;

#[derive(ModuleInfo, Clone)]
pub struct Evm<C: sov_modules_api::Context> {
    #[address]
    pub(crate) address: C::Address,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Evm<C> {
    type Context = C;

    type Config = ();

    type CallMessage = call::CallMessage;

    fn genesis(
        &self,
        _config: &Self::Config,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        todo!()
    }

    fn call(
        &self,
        _msg: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        todo!()
    }
}
