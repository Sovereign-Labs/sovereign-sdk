use std::convert::Infallible;

use reth_revm::tracing::{TracingInspector, TracingInspectorConfig};
use revm::primitives::{CfgEnv, EVMError, Env, ExecutionResult, ResultAndState, TxEnv};
use revm::{self, Database, DatabaseCommit};

use super::transaction::{BlockEnv, EvmTransactionSignedEcRecovered};

pub(crate) fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block_env: BlockEnv,
    tx: &EvmTransactionSignedEcRecovered,
    config_env: CfgEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();

    let env = Env {
        block: block_env.into(),
        cfg: config_env,
        tx: tx.into(),
    };

    evm.env = env;
    evm.database(db);
    evm.transact_commit()
}

pub(crate) fn inspect<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block_env: BlockEnv,
    tx: TxEnv,
    config_env: CfgEnv,
) -> Result<ResultAndState, EVMError<Infallible>> {
    let mut evm = revm::new();

    let env = Env {
        cfg: config_env,
        block: block_env.into(),
        tx,
    };

    evm.env = env;
    evm.database(db);

    let config = TracingInspectorConfig::all();

    let mut inspector = TracingInspector::new(config);

    evm.inspect(&mut inspector)
}
