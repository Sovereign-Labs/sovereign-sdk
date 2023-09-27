use std::convert::Infallible;

use reth_primitives::TransactionKind;
use revm::db::CacheDB;
use revm::precompile::B160;
use revm::primitives::{CfgEnv, ExecutionResult, Output, SpecId, KECCAK_EMPTY, U256};
use revm::{Database, DatabaseCommit};
use sov_modules_api::WorkingSet;
use sov_state::ProverStorage;

use super::db::EvmDb;
use super::db_init::InitEvmDb;
use super::executor;
use crate::evm::primitive_types::BlockEnv;
use crate::evm::AccountInfo;
use crate::smart_contracts::SimpleStorageContract;
use crate::tests::dev_signer::TestSigner;
use crate::Evm;
type C = sov_modules_api::default_context::DefaultContext;

pub(crate) fn output(result: ExecutionResult) -> bytes::Bytes {
    match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(out) => out,
            Output::Create(out, _) => out,
        },
        _ => panic!("Expected successful ExecutionResult"),
    }
}

#[test]
fn simple_contract_execution_sov_state() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set: WorkingSet<C> =
        WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    let evm = Evm::<C>::default();
    let evm_db: EvmDb<'_, C> = evm.get_db(&mut working_set);

    simple_contract_execution(evm_db);
}

#[test]
fn simple_contract_execution_in_memory_state() {
    let db = CacheDB::default();
    simple_contract_execution(db);
}

fn simple_contract_execution<DB: Database<Error = Infallible> + DatabaseCommit + InitEvmDb>(
    mut evm_db: DB,
) {
    let dev_signer = TestSigner::new_random();
    let caller = dev_signer.address();
    evm_db.insert_account_info(
        caller,
        AccountInfo {
            balance: U256::from(1000000000),
            code_hash: KECCAK_EMPTY,
            nonce: 1,
        },
    );

    let contract = SimpleStorageContract::default();

    // We are not supporting CANCUN yet
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
    let mut cfg_env = CfgEnv::default();
    cfg_env.spec_id = SpecId::SHANGHAI;

    let contract_address: B160 = {
        let tx = dev_signer
            .sign_default_transaction(TransactionKind::Create, contract.byte_code().to_vec(), 1)
            .unwrap();

        let tx = &tx.try_into().unwrap();
        let block_env = BlockEnv {
            gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
            ..Default::default()
        };

        let result = executor::execute_tx(&mut evm_db, &block_env, tx, cfg_env.clone()).unwrap();
        contract_address(&result).expect("Expected successful contract creation")
    };

    let set_arg = 21989;

    {
        let call_data = contract.set_call_data(set_arg);

        let tx = dev_signer
            .sign_default_transaction(
                TransactionKind::Call(contract_address.into()),
                hex::decode(hex::encode(&call_data)).unwrap(),
                2,
            )
            .unwrap();

        let tx = &tx.try_into().unwrap();
        executor::execute_tx(&mut evm_db, &BlockEnv::default(), tx, cfg_env.clone()).unwrap();
    }

    let get_res = {
        let call_data = contract.get_call_data();

        let tx = dev_signer
            .sign_default_transaction(
                TransactionKind::Call(contract_address.into()),
                hex::decode(hex::encode(&call_data)).unwrap(),
                3,
            )
            .unwrap();

        let tx = &tx.try_into().unwrap();
        let result =
            executor::execute_tx(&mut evm_db, &BlockEnv::default(), tx, cfg_env.clone()).unwrap();

        let out = output(result);
        ethereum_types::U256::from(out.as_ref())
    };

    assert_eq!(set_arg, get_res.as_u32());

    {
        let failing_call_data = contract.failing_function_call_data();

        let tx = dev_signer
            .sign_default_transaction(
                TransactionKind::Call(contract_address.into()),
                hex::decode(hex::encode(&failing_call_data)).unwrap(),
                4,
            )
            .unwrap();

        let tx = &tx.try_into().unwrap();
        let result =
            executor::execute_tx(&mut evm_db, &BlockEnv::default(), tx, cfg_env.clone()).unwrap();

        assert!(matches!(result, ExecutionResult::Revert { .. }));
    }
}

fn contract_address(result: &ExecutionResult) -> Option<B160> {
    match result {
        ExecutionResult::Success {
            output: Output::Create(_, Some(addr)),
            ..
        } => Some(**addr),
        _ => None,
    }
}
