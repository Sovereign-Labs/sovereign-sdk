use crate::helpers::*;
use crate::runtime::RT;
use crate::runtime::S;
use alloy_primitives::FixedBytes;
use alloy_primitives::U256;
use alloy_rpc_types::BlockTransactions;
use reth_primitives::Log;
use revm::Database;
use sov_evm::Evm;
use sov_modules_api::GasArray;
use sov_test_utils::TransactionType;
use sov_test_utils::{BatchTestCase, SimpleStorageContract};
use sov_test_utils::{TransactionTestCase, TEST_DEFAULT_USER_BALANCE};

#[test]
fn test_simple_transfer() {
    let (mut runner, from, to) = setup();

    let value = 1;
    let transfer_tx = create_transfer_tx(0, &from, &to, value).tx;

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
fn test_evm_gas_usage() {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_DEFAULT_GAS_TO_CHARGE_PER_EVM_GAS",
        "[2, 0]",
    );
    let gas_used_with_evm_metering = {
        let (mut runner, from, _) = setup();
        let contract = SimpleStorageContract::default();
        let contract_addr = from.address().create(0);
        runner.execute(create_deploy_tx(0, &contract, &from).tx);
        let transfer = create_set_arg_tx(0, 1, &contract, contract_addr, &from).tx;
        let (receipt, _) = runner.execute(transfer);
        receipt.last_batch_receipt().inner.gas_used.clone()
    };
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_DEFAULT_GAS_TO_CHARGE_PER_EVM_GAS",
        "[1, 0]",
    );
    let gas_used_without_evm_metering = {
        let (mut runner, from, _) = setup();
        let contract = SimpleStorageContract::default();
        let contract_addr = from.address().create(0);
        runner.execute(create_deploy_tx(0, &contract, &from).tx);
        let transfer = create_set_arg_tx(0, 1, &contract, contract_addr, &from).tx;
        let (receipt, _) = runner.execute(transfer);
        receipt.last_batch_receipt().inner.gas_used.clone()
    };
    assert_eq!(
        gas_used_with_evm_metering
            .checked_sub(&gas_used_without_evm_metering)
            .unwrap()
            .as_ref(),
        &[4_323, 0]
    );
}

#[test]
fn test_executing_eth_transactions() {
    let (mut runner, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr = account.address().create(0);

    let address = account.address();
    let mut txs = vec![create_deploy_tx(0, &contract, &account)];

    for nonce in 1..10 {
        txs.push(create_set_arg_tx(
            (nonce + 100) as u32, // Add 100 so that the value set in the smart contract differs from the nonce.
            nonce,
            &contract,
            contract_addr,
            &account,
        ));
    }

    for tx in txs {
        let evm = Evm::<S>::default();
        runner.execute_transaction(TransactionTestCase {
            input: tx.tx,
            assert: Box::new(move |_result, state| {
                let nonce = tx.nonce;
                let tx_hash = tx.hash;

                assert_eq!(evm.pending_transactions(state).len(), 0);

                assert_eq!(
                    evm.transaction(nonce, state)
                        .unwrap()
                        .signed_transaction()
                        .hash(),
                    &tx_hash
                );

                assert_eq!(evm.get_tx_index_by_hash(&tx_hash, state), Some(nonce));

                assert!(evm.receipt(nonce, state).unwrap().receipt.success);

                let nonce_from_module = evm
                    .get_transaction_count(address, None, state)
                    .unwrap()
                    .to::<u64>();

                assert_eq!(nonce + 1, nonce_from_module);

                let storage_value = evm.get_storage(&contract_addr, &U256::ZERO, state).unwrap();

                if nonce == 0 {
                    // On contract creation the value is absent.
                    assert!(storage_value.is_none());
                } else {
                    assert_eq!(U256::from(nonce + 100), storage_value.unwrap());
                }
            }),
        });
    }
}

#[test]
fn test_executing_eth_transactions_several_blocks() {
    let (mut runner, from, to) = setup();

    let nb_of_transfers: u64 = 200;
    let batch_size: usize = 10;

    let blocks = Block::create_blocks(nb_of_transfers, batch_size, &from, &to);

    // Execute all the batches
    for block in blocks {
        let evm = Evm::<S>::default();

        runner.execute_batch(BatchTestCase {
            input: block.batch_txs().into(),
            assert: Box::new(move |_result, state| {
                assert_eq!(block.nr, evm.block_number(state).unwrap().to::<u64>());
                let block_from_evm = evm.get_block_by_number(None, None, state).unwrap().unwrap();

                if let BlockTransactions::Hashes(hashes) = &block_from_evm.transactions {
                    assert_eq!(hashes, &block.tx_hashes());
                } else {
                    panic!("The test expects BlockTransactions::Hashes");
                }
                assert_eq!(batch_size, block.transactions.len());

                for (tx_index, tx) in block.transactions.iter().enumerate() {
                    let tx_index = tx_index as u64;

                    let tx_from_evm = evm
                        .get_transaction_by_hash(tx.hash, state)
                        .unwrap()
                        .unwrap();

                    assert_eq!(&tx.hash, tx_from_evm.inner.hash());
                    assert_eq!(tx_index, tx_from_evm.transaction_index.unwrap());
                    assert_eq!(block.nr, tx_from_evm.block_number.unwrap());

                    let receipt_from_evm = evm
                        .get_transaction_receipt(tx.hash, state)
                        .unwrap()
                        .unwrap();

                    assert_eq!(tx.hash, receipt_from_evm.transaction_hash);
                    assert_eq!(block.nr, receipt_from_evm.block_number.unwrap());
                    assert_eq!(tx_index, receipt_from_evm.transaction_index.unwrap());
                }
            }),
        });
    }
}

#[test]
fn test_failed_tx_doesnt_update_evm_module_state() {
    let (mut runner, _, no_balance_account) = setup();
    let contract = SimpleStorageContract::default();
    let create_contract_tx = create_deploy_tx(0, &contract, &no_balance_account).tx;

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
    let transfer_tx = create_transfer_tx(0, &from, &to, value).tx;

    let evm = Evm::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: transfer_tx,
        assert: Box::new(move |_ctx, state| {
            let mut db = evm.get_db(state);
            let from_acc = db.basic(from_addr).unwrap().unwrap();
            assert_eq!(from_acc.nonce, 1);
        }),
    });

    let transfer_tx = create_transfer_tx(1, &from, &to, value).tx;

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

    let create_contract_tx_1 = create_deploy_tx(0, &contract, &account).tx;
    let set_value_tx_1 = create_set_arg_tx(1, 1, &contract, contract_addr_1, &account).tx;

    let contract_addr_2 = account.address().create(2);
    let create_contract_tx_2 = create_deploy_tx(2, &contract, &account).tx;
    let set_value_tx_2 = create_set_arg_tx(2, 3, &contract, contract_addr_2, &account).tx;

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

#[test]
fn test_evm_logs() {
    let (mut runner, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr = account.address().create(0);
    let address_bytes: [u8; 32] = account.address().into_word().into();

    let check_logs = move |logs: &[Log], len: usize| {
        assert_eq!(logs.len(), len);

        for log in logs {
            assert_eq!(log.topics().len(), 2);
            assert_eq!(log.address, contract_addr);
            assert_eq!(log.topics()[1], address_bytes);
        }
    };

    let mut txs = vec![create_deploy_tx(0, &contract, &account).tx];

    txs.push(create_emit_one_log(1, &contract, contract_addr, &account).tx);
    txs.push(create_emit_two_logs(2, &contract, contract_addr, &account).tx);

    let evm = Evm::<S>::default();

    runner.execute_batch(BatchTestCase {
        input: txs.into(),
        assert: Box::new(move |_result, state| {
            let logs_1 = evm.receipt(1, state).unwrap().receipt.logs;
            check_logs(&logs_1, 1);

            let logs_2 = evm.receipt(2, state).unwrap().receipt.logs;
            check_logs(&logs_2, 2);
        }),
    });
}

struct Block {
    nr: u64,
    transactions: Vec<TxWithNonceAndHash>,
}

impl Block {
    fn create_blocks(
        nb_of_transfers: u64,
        batch_size: usize,
        from: &EvmAccount,
        to: &EvmAccount,
    ) -> Vec<Block> {
        let value = 1;
        // 1. Create `batch_size`` transfers
        let transfers: Vec<TxWithNonceAndHash> = (0..nb_of_transfers)
            .map(|nonce| create_transfer_tx(nonce, from, to, value))
            .collect();

        let mut blocks = vec![];

        // We start from 1 becaue genesis is alredy in the state.
        let mut nr = 1;
        for txs in transfers.chunks(batch_size) {
            blocks.push(Block {
                nr,
                transactions: txs.to_vec(),
            });
            nr += 1;
        }

        blocks
    }

    fn batch_txs(&self) -> Vec<TransactionType<RT, S>> {
        self.transactions.iter().cloned().map(|tx| tx.tx).collect()
    }

    fn tx_hashes(&self) -> Vec<FixedBytes<32>> {
        self.transactions.iter().map(|tx| tx.hash).collect()
    }
}
