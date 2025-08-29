use crate::helpers::*;
use crate::runtime::S;
use alloy_primitives::U256;
use revm::Database;
use sov_evm::Evm;
use sov_test_utils::{BatchTestCase, SimpleStorageContract};
use sov_test_utils::{TransactionTestCase, TEST_DEFAULT_USER_BALANCE};

#[test]
fn test_simple_transfer() {
    let (mut runner, from, to) = setup();

    let value = 1;
    let transfer_tx = create_transfer_tx(0, &from, &to, value);

    let evm = Evm::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: transfer_tx,
        assert: Box::new(move |ctx, state| {
            let mut db = evm.get_db(state);
            let from_acc = db.basic(from.address()).unwrap().unwrap();
            let to_acc = db.basic(to.address()).unwrap().unwrap();
            // The only balance changes should be from the trasfer itself and not from gas as it's disabled in SovEvm
            assert_eq!(
                from_acc.balance,
                TEST_DEFAULT_USER_BALANCE.0 - value - ctx.gas_value_used.0
            );
            assert_eq!(to_acc.balance, value);
        }),
    });
}

#[test]
fn test_executing_eth_transactions() {
    let (mut runner, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr = account.address().create(0);

    let create_contract_tx = create_deploy_tx(0, &contract, &account);
    let set_value_tx = create_set_arg_tx(5, 1, &contract, contract_addr, &account);

    runner.execute_batch(BatchTestCase {
        input: vec![create_contract_tx, set_value_tx].into(),
        assert: Box::new(move |_result, state| {
            let evm = Evm::<S>::default();
            let receipts = evm.receipts(state);
            assert_eq!(receipts.len(), 2);
            for receipt in receipts {
                assert!(
                    receipt.receipt.success,
                    "Eth tx didn't execute successfully, receipt: {receipt:?}"
                );
            }
            let storage_value = evm
                .get_storage(&contract_addr, &U256::ZERO, state)
                .unwrap()
                .unwrap();
            assert_eq!(U256::from(5), storage_value);
        }),
    });

    for n in 2..10 {
        let address = account.address();
        let set_value_tx =
            create_set_arg_tx((n + 90) as u32, n, &contract, contract_addr, &account);

        runner.execute_batch(BatchTestCase {
            input: vec![set_value_tx].into(),
            assert: Box::new(move |_result, state| {
                let evm = Evm::<S>::default();
                let nonce_from_module = evm
                    .get_transaction_count(address, None, state)
                    .unwrap()
                    .to::<u64>();
                assert_eq!(n + 1, nonce_from_module);

                let storage_value = evm
                    .get_storage(&contract_addr, &U256::ZERO, state)
                    .unwrap()
                    .unwrap();
                assert_eq!(U256::from(n + 90), storage_value);
            }),
        });
    }
}

#[test]
fn test_failed_tx_doesnt_update_evm_module_state() {
    let (mut runner, _, no_balance_account) = setup();
    let contract = SimpleStorageContract::default();
    let create_contract_tx = create_deploy_tx(0, &contract, &no_balance_account);

    runner.execute_batch(BatchTestCase {
        input: vec![create_contract_tx].into(),
        assert: Box::new(move |_result, state| {
            let evm = Evm::<S>::default();
            // no pending block added if eth tx execution fails.
            assert!(evm.pending_head(state).is_none());
            assert!(evm.pending_transactions(state).is_empty());
        }),
    });
}

#[test]
fn test_account_nonce() {
    let (mut runner, from, to) = setup();

    let from_addr = from.address();
    let value = 1;
    let transfer_tx = create_transfer_tx(0, &from, &to, value);

    let evm = Evm::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: transfer_tx,
        assert: Box::new(move |_ctx, state| {
            let mut db = evm.get_db(state);
            let from_acc = db.basic(from_addr).unwrap().unwrap();
            assert_eq!(from_acc.nonce, 1);
        }),
    });

    let transfer_tx = create_transfer_tx(1, &from, &to, value);

    let evm = Evm::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: transfer_tx,
        assert: Box::new(move |_ctx, state| {
            let mut db = evm.get_db(state);
            let from_acc = db.basic(from_addr).unwrap().unwrap();
            assert_eq!(from_acc.nonce, 2);
        }),
    });
}

// Check that if the same account deploys two contracts, each deployment results in a unique contract address
#[test]
fn test_deploy_many_contracts() {
    let (mut runner, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr_1 = account.address().create(0);

    let create_contract_tx_1 = create_deploy_tx(0, &contract, &account);
    let set_value_tx_1 = create_set_arg_tx(1, 1, &contract, contract_addr_1, &account);

    let contract_addr_2 = account.address().create(2);
    let create_contract_tx_2 = create_deploy_tx(2, &contract, &account);
    let set_value_tx_2 = create_set_arg_tx(2, 3, &contract, contract_addr_2, &account);

    let evm = Evm::<S>::default();
    runner.execute_batch(BatchTestCase {
        input: vec![
            // Deploy a contract and execute single transaction that update its storage.
            create_contract_tx_1,
            set_value_tx_1,
            // Deploy another contract and execut single transaction that updates its storage.
            create_contract_tx_2,
            set_value_tx_2,
        ]
        .into(),
        assert: Box::new(move |_ctx, state| {
            // The two contracts have different addresses.
            assert_ne!(contract_addr_1, contract_addr_2);

            let mut db = evm.get_db(state);
            let contract_1_account = db.basic(contract_addr_1).unwrap().unwrap();
            let contract_2_account = db.basic(contract_addr_2).unwrap().unwrap();

            // The two contracts have the same code.
            assert_eq!(contract_1_account.code_hash, contract_2_account.code_hash);

            let storage_value_2 = evm
                .get_storage(&contract_addr_2, &U256::ZERO, state)
                .unwrap()
                .unwrap();

            assert_eq!(U256::from(2), storage_value_2);

            // The storage of the first contract didn't change.
            let storage_value_1 = evm
                .get_storage(&contract_addr_1, &U256::ZERO, state)
                .unwrap()
                .unwrap();

            assert_eq!(U256::from(1), storage_value_1);
        }),
    });
}
