use super::transaction::{BlockEnv, EvmTransaction};
use revm::{
    self,
    primitives::{CfgEnv, EVMError, Env, ExecutionResult},
    Database, DatabaseCommit,
};
use std::convert::Infallible;

#[allow(dead_code)]
pub(crate) fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block_env: BlockEnv,
    tx: EvmTransaction,
    config_env: CfgEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();

    let env = Env {
        cfg: config_env,
        block: block_env.into(),
        tx: tx.into(),
    };

    evm.env = env;
    evm.database(db);
    evm.transact_commit()
}
