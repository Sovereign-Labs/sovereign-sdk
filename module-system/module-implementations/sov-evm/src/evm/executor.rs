use std::convert::Infallible;

use ethers_core::types::{Block, Transaction, TxHash};
use reth_revm::tracing::{TracingInspector, TracingInspectorConfig};
use revm::primitives::{
    BlockEnv, CfgEnv, EVMError, Env, ExecutionResult, ResultAndState, TransactTo, TxEnv,
};
use revm::{self, Database, DatabaseCommit};

use super::transaction::effective_gas_price;

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
        tx: convert_transaction(tx, block),
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

fn convert_transaction(tx: &Transaction, block: &Block<TxHash>) -> TxEnv {
    TxEnv {
        caller: tx.from.into(),
        gas_limit: tx.gas.as_u64(),
        gas_price: effective_gas_price(tx, block.base_fee_per_gas).into(),
        gas_priority_fee: tx.max_fee_per_gas.map(|gas| gas.into()),
        transact_to: match tx.to {
            Some(to) => TransactTo::Call(to.into()),
            None => TransactTo::Create(revm::primitives::CreateScheme::Create),
        },
        value: tx.value.into(),
        data: tx.input.clone().0,
        chain_id: tx.chain_id.map(|id| id.as_u64()),
        nonce: Some(tx.nonce.as_u64()),
        access_list: vec![], // TODO: implement
    }
}

fn convert_block(block: &Block<TxHash>) -> BlockEnv {
    BlockEnv {
        gas_limit: revm::primitives::U256::MAX, // block.gas_limit.into(),
        number: revm::primitives::U256::from(block.number.unwrap_or_default().as_u64()),
        coinbase: block.author.unwrap_or_default().into(),
        timestamp: block.timestamp.into(),
        difficulty: block.difficulty.into(),
        prevrandao: block
            .mix_hash
            .map_or(Some(Default::default()), |hash| Some(hash.into())),
        basefee: block.base_fee_per_gas.unwrap_or_default().into(),
    }
}
