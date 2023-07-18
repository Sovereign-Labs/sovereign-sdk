use std::convert::Infallible;

use revm::db::CacheDB;
use revm::primitives::{CfgEnv, KECCAK_EMPTY, U256};
use revm::{Database, DatabaseCommit};
use sov_state::{ProverStorage, WorkingSet};

use super::db::EvmDb;
use super::db_init::InitEvmDb;
use super::executor;
use crate::evm::test_helpers::{output, SimpleStorageContract};
use crate::evm::transaction::{BlockEnv, EvmTransaction};
use crate::evm::{contract_address, AccountInfo};
use crate::Evm;

type C = sov_modules_api::default_context::DefaultContext;

#[test]
fn simple_contract_execution_sov_state() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set: WorkingSet<<C as sov_modules_api::Spec>::Storage> =
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
    let caller: [u8; 20] = [11; 20];
    evm_db.insert_account_info(
        caller,
        AccountInfo {
            balance: U256::from(1000000000).to_le_bytes(),
            code_hash: KECCAK_EMPTY.to_fixed_bytes(),
            code: vec![],
            nonce: 1,
        },
    );

    let contract = SimpleStorageContract::new();

    let contract_address = {
        let tx = EvmTransaction {
            to: None,
            data: contract.byte_code().to_vec(),
            ..Default::default()
        };

        let result =
            executor::execute_tx(&mut evm_db, BlockEnv::default(), tx, CfgEnv::default()).unwrap();
        contract_address(result).expect("Expected successful contract creation")
    };

    let set_arg = 21989;

    {
        let call_data = contract.set_call_data(set_arg);

        let tx = EvmTransaction {
            to: Some(*contract_address.as_fixed_bytes()),
            data: hex::decode(hex::encode(&call_data)).unwrap(),
            nonce: 1,
            ..Default::default()
        };

        executor::execute_tx(&mut evm_db, BlockEnv::default(), tx, CfgEnv::default()).unwrap();
    }

    let get_res = {
        let call_data = contract.get_call_data();

        let tx = EvmTransaction {
            to: Some(*contract_address.as_fixed_bytes()),
            data: hex::decode(hex::encode(&call_data)).unwrap(),
            nonce: 2,
            ..Default::default()
        };

        let result =
            executor::execute_tx(&mut evm_db, BlockEnv::default(), tx, CfgEnv::default()).unwrap();

        let out = output(result);
        ethereum_types::U256::from(out.as_ref())
    };

    assert_eq!(set_arg, get_res.as_u32())
}
