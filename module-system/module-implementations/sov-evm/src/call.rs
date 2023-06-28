use crate::{
    evm::{
        db::EvmDb,
        executor::{self},
    },
    Evm,
};
use anyhow::Result;

use revm::primitives::{CfgEnv, ExecutionResult};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct CallMessage {
    pub tx: executor::EvmTransaction,
}

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn execute_call(
        &self,
        tx: executor::EvmTransaction,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.execute_tx(tx, context, working_set).unwrap();
        Ok(CallResponse::default())
    }

    pub(crate) fn execute_tx(
        &self,
        tx: executor::EvmTransaction,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<ExecutionResult> {
        let cfg_env = CfgEnv::default();
        let block_env = self.block_env.get(working_set).unwrap_or_default();
        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        Ok(executor::execute_tx(evm_db, block_env, tx, cfg_env).unwrap())
    }
}
