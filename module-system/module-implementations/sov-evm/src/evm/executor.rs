use revm::{
    self,
    primitives::{EVMError, Env, ExecutionResult},
    Database, DatabaseCommit,
};
use std::convert::Infallible;

#[allow(dead_code)]
pub(crate) fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    env: Env,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();
    evm.env = env;
    evm.database(db);
    evm.transact_commit()
}
