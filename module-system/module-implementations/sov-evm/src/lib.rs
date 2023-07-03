use evm::{db::EvmDb, transaction::BlockEnv, DbAccount, EthAddress};
use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use sov_state::WorkingSet;

pub mod call;
mod evm;
pub mod genesis;
#[cfg(feature = "native")]
pub mod query;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tx_tests;

#[allow(dead_code)]
#[derive(ModuleInfo, Clone)]
pub struct Evm<C: sov_modules_api::Context> {
    #[address]
    pub(crate) address: C::Address,

    #[state]
    pub(crate) accounts: sov_state::StateMap<EthAddress, DbAccount>,

    #[state]
    pub(crate) block_env: sov_state::StateValue<BlockEnv>,
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
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        Ok(self.execute_call(msg.tx, context, working_set)?)
    }
}

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn get_db<'a>(&self, working_set: &'a mut WorkingSet<C::Storage>) -> EvmDb<'a, C> {
        EvmDb::new(self.accounts.clone(), working_set)
    }
}
