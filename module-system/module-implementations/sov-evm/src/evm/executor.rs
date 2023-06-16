use std::convert::Infallible;

use revm::{
    self,
    primitives::{EVMError, ExecutionResult, TxEnv},
    DummyStateDB,
};

pub(crate) fn execute_tx(
    db: DummyStateDB,
    tx_env: TxEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();
    evm.env.tx = tx_env;
    evm.database(db);
    evm.transact_commit()
}
