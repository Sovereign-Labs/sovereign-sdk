use reth_primitives::{TxKind, U256};
use reth_rpc_types::transaction::EIP1559TransactionRequest;
use reth_rpc_types::TypedTransactionRequest;
use sov_evm::{EthereumAuthenticator, Evm};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::{BatchTestCase, SimpleStorageContract, TransactionType};

use crate::helpers::setup;
use crate::runtime::{RT, S};

#[test]
fn test_executing_eth_transaction() {
    let (mut runner, _, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr = account.address().create(0);
    let create_contract_tx_request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 0,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: U256::from(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2),
        gas_limit: U256::from(1_000_000u64),
        kind: TxKind::Create,
        value: Default::default(),
        input: reth_primitives::Bytes::from(contract.byte_code().to_vec()),
        access_list: Default::default(),
    });
    let (signed_eth_tx, _) = account.sign(create_contract_tx_request);
    let create_contract_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };

    let set_arg_eth_tx = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 1,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: U256::from(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2),
        gas_limit: U256::from(1_000_000u64),
        kind: TxKind::Call(contract_addr),
        value: Default::default(),
        input: reth_primitives::Bytes::from(
            hex::decode(hex::encode(contract.set_call_data(5))).unwrap(),
        ),
        access_list: Default::default(),
    });
    let (signed_eth_tx, _) = account.sign(set_arg_eth_tx);
    let set_value_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };

    runner.execute_batch(BatchTestCase {
        input: vec![
            TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(
                create_contract_tx,
            )),
            TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(set_value_tx)),
        ]
        .into(),
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

    let set_arg_eth_tx = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 2,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: U256::from(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2),
        gas_limit: U256::from(1_000_000u64),
        kind: TxKind::Call(contract_addr),
        value: Default::default(),
        input: reth_primitives::Bytes::from(
            hex::decode(hex::encode(contract.set_call_data(92))).unwrap(),
        ),
        access_list: Default::default(),
    });
    let (signed_eth_tx, _) = account.sign(set_arg_eth_tx);
    let set_value_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };

    runner.execute_batch(BatchTestCase {
        input: vec![TransactionType::<RT, S>::PreAuthenticated(
            RT::encode_with_ethereum_auth(set_value_tx),
        )]
        .into(),
        assert: Box::new(move |_result, state| {
            let evm = Evm::<S>::default();
            let storage_value = evm
                .get_storage(&contract_addr, &U256::ZERO, state)
                .unwrap()
                .unwrap();
            assert_eq!(U256::from(92), storage_value);
        }),
    });
}

#[test]
fn test_failed_tx_doesnt_update_evm_module_state() {
    let (mut runner, _, _, no_balance_account) = setup();
    let contract = SimpleStorageContract::default();
    let create_contract_tx_request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 0,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: U256::from(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2),
        gas_limit: U256::from(1_000_000u64),
        kind: TxKind::Create,
        value: Default::default(),
        input: reth_primitives::Bytes::from(contract.byte_code().to_vec()),
        access_list: Default::default(),
    });
    let (signed_eth_tx, _) = no_balance_account.sign(create_contract_tx_request);
    let create_contract_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };
    runner.execute_batch(BatchTestCase {
        input: vec![TransactionType::<RT, S>::PreAuthenticated(
            RT::encode_with_ethereum_auth(create_contract_tx),
        )]
        .into(),
        assert: Box::new(move |_result, state| {
            let evm = Evm::<S>::default();
            // no pending block added if eth tx execution fails.
            assert!(evm.pending_head(state).is_none());
            assert!(evm.pending_transactions(state).is_empty());
        }),
    });
}
