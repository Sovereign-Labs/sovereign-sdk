use super::db::EvmDb;
use std::convert::Infallible;

use revm::{
    self,
    primitives::{EVMError, ExecutionResult, TxEnv},
};

#[allow(dead_code)]
pub(crate) fn execute_tx(
    db: EvmDb,
    tx_env: TxEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();
    evm.env.tx = tx_env;
    evm.database(db);
    evm.transact_commit()
}
