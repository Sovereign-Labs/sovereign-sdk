use revm::{
    self,
    primitives::{EVMError, ExecutionResult, TxEnv},
    Database, DatabaseCommit,
};
use std::convert::Infallible;

#[allow(dead_code)]
pub(crate) fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    tx_env: TxEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();
    evm.env.tx = tx_env;
    evm.database(db);
    evm.transact_commit()
}
