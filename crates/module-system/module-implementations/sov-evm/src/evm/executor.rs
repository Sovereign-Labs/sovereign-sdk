use std::convert::Infallible;

use reth_primitives::revm_primitives::{
    Address, BlockEnv, CfgEnvWithHandlerCfg, EVMError, Env, EnvWithHandlerCfg, ExecutionResult,
};
use reth_primitives::TransactionSigned;
use revm::{Database, DatabaseCommit, EvmBuilder};

use crate::evm::conversions::create_tx_env;

/// Execute an Ethereum transaction and commit it to the database.
pub fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block_env: &BlockEnv,
    tx: &TransactionSigned,
    signer: Address,
    config_env: CfgEnvWithHandlerCfg,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let CfgEnvWithHandlerCfg {
        cfg_env,
        handler_cfg,
    } = config_env;

    let env_with_handler_cfg = EnvWithHandlerCfg {
        env: Box::new(Env {
            cfg: cfg_env,
            block: block_env.clone(),
            tx: create_tx_env(tx, signer),
        }),
        handler_cfg,
    };

    let mut evm = EvmBuilder::default()
        .with_db(db)
        .with_env_with_handler_cfg(env_with_handler_cfg)
        .build();

    evm.transact_commit()
}

#[cfg(feature = "native")]
pub(crate) fn inspect<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block_env: &BlockEnv,
    tx: reth_primitives::revm_primitives::TxEnv,
    config_env: CfgEnvWithHandlerCfg,
) -> Result<reth_primitives::revm_primitives::ResultAndState, EVMError<Infallible>> {
    let CfgEnvWithHandlerCfg {
        cfg_env,
        handler_cfg,
    } = config_env;

    let env_with_handler_cfg = EnvWithHandlerCfg {
        env: Box::new(Env {
            cfg: cfg_env,
            block: block_env.clone(),
            tx,
        }),
        handler_cfg,
    };

    let config = revm_inspectors::tracing::TracingInspectorConfig::all();
    let mut inspector = revm_inspectors::tracing::TracingInspector::new(config);

    let mut evm = EvmBuilder::default()
        .with_external_context(&mut inspector)
        .with_db(db)
        .with_env_with_handler_cfg(env_with_handler_cfg)
        .build();

    evm.transact()
}
