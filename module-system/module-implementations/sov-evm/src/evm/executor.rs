use std::convert::Infallible;

use ethers_core::types::{Block, Transaction, TxHash};
use reth_primitives::U256;
use reth_revm::tracing::{TracingInspector, TracingInspectorConfig};
use revm::primitives::{
    BlockEnv, CfgEnv, EVMError, Env, ExecutionResult, ResultAndState, TransactTo, TxEnv,
};
use revm::{self, Database, DatabaseCommit};

use super::transaction::EvmTransaction;

pub(crate) fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block: &Block<TxHash>,
    tx: &Transaction,
    config_env: CfgEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();
    let block_env = convert_block(block);

    let env = Env {
        block: block_env,
        cfg: config_env,
        tx: convert_transaction(tx, &block_env),
    };

    evm.env = env;
    evm.database(db);
    evm.transact_commit()
}

pub(crate) fn inspect<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block: &Block<TxHash>,
    tx: TxEnv,
    config_env: CfgEnv,
) -> Result<ResultAndState, EVMError<Infallible>> {
    let mut evm = revm::new();

    let env = Env {
        cfg: config_env,
        block: convert_block(block),
        tx,
    };

    evm.env = env;
    evm.database(db);

    let config = TracingInspectorConfig::all();

    let mut inspector = TracingInspector::new(config);

    evm.inspect(&mut inspector)
}

fn convert_transaction(tx: &Transaction, block: &BlockEnv) -> TxEnv {
    TxEnv {
        caller: tx.from.into(),
        gas_limit: tx.gas_price.unwrap().as_u64(),
        gas_price: U256::from(tx.effective_gas_price(block.basefee)),
        gas_priority_fee: Default::default(),
        //tx.max_fee_per_gas.map(|gas| U256::from(gas)),
        transact_to: TransactTo::Call(tx.to.unwrap().into()),
        value: Default::default(),
        data: Default::default(),
        chain_id: Default::default(),
        nonce: Default::default(),
        access_list: Default::default(),
    }
}

fn convert_block(block: &Block<TxHash>) -> BlockEnv {
    BlockEnv {
        gas_limit: block.gas_limit.into(),
        number: U256::from(block.number.unwrap_or_default().as_u64()),
        coinbase: block.author.unwrap().into(),
        timestamp: block.timestamp.into(),
        difficulty: block.difficulty.into(),
        prevrandao: block.mix_hash.map(|hash| hash.into()),
        basefee: block.base_fee_per_gas.unwrap_or_default().into(),
    }
}
