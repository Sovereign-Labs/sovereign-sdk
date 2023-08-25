use sov_modules_api::{CallResponse, Context, Error, Module, ModuleInfo};
use sov_state::{StateValue, WorkingSet};

#[derive(ModuleInfo)]
pub struct QueryModule<C: Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub data: StateValue<u8>,
}

impl<C: Context> Module for QueryModule<C> {
    type Context = C;
    type Config = u8;
    type CallMessage = u8;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        self.data.set(config, working_set);
        Ok(())
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        _context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse, Error> {
        self.data.set(&msg, working_set);
        Ok(CallResponse::default())
    }
}
fn main() {}
