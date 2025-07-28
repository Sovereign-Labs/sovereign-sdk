use reth_primitives::{TxKind, U256};
use reth_rpc_types::transaction::EIP1559TransactionRequest;
use reth_rpc_types::TypedTransactionRequest;
use sov_evm::{EthereumAuthenticator, Evm};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::{BatchTestCase, SimpleStorageContract, TransactionTestCase, TransactionType};

use crate::helpers::setup;
use crate::runtime::{RT, S};

#[test]
fn test_block_updates() {
    let (mut runner, _, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let create_contract_tx_request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 0,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: reth_primitives::U256::from(
            reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2,
        ),
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

    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(
            create_contract_tx,
        )),
        assert: Box::new(move |_result, state| {
            let evm = Evm::<S>::default();
            assert!(evm.pending_head(state).is_none());
            let blocks = evm.blocks(state);
            let prev_block = &blocks[0];
            let current_block = &blocks[1];
            assert!(prev_block != current_block,);
            assert_eq!(
                current_block.header().parent_hash,
                prev_block.header().hash()
            );
            assert!(!current_block.header().transaction_root_is_empty(),);
            assert_eq!(current_block.header().number, 1);
            let txs = current_block.transactions();
            assert_eq!(txs.start, 0);
            assert_eq!(txs.end, 1);
            let block_height = evm.get_block_height_by_hash(&current_block.header().hash(), state);
            assert_eq!(block_height, Some(1));
        }),
    });
}

#[test]
fn test_transactions_receipts() {
    let (mut runner, _, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let tx_1_request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 0,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: reth_primitives::U256::from(
            reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2,
        ),
        gas_limit: U256::from(1_000_000u64),
        kind: TxKind::Create,
        value: Default::default(),
        input: reth_primitives::Bytes::from(contract.byte_code().to_vec()),
        access_list: Default::default(),
    });
    let (rlp_tx, signed_tx1) = account.sign(tx_1_request);
    let tx_1 = RawTx {
        data: borsh::to_vec(&rlp_tx).unwrap(),
    };
    let tx_2_request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 1,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: reth_primitives::U256::from(
            reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2,
        ),
        gas_limit: U256::from(1_000_000u64),
        kind: TxKind::Create,
        value: Default::default(),
        input: reth_primitives::Bytes::from(contract.byte_code().to_vec()),
        access_list: Default::default(),
    });
    let (rlp_tx, signed_tx2) = account.sign(tx_2_request);
    let tx_2 = RawTx {
        data: borsh::to_vec(&rlp_tx).unwrap(),
    };

    runner.execute_batch(BatchTestCase {
        input: vec![
            TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(tx_1)),
            TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(tx_2)),
        ]
        .into(),
        assert: Box::new(move |_result, state| {
            let evm = Evm::<S>::default();
            let txns = evm.transactions(state);
            let signed_txns = txns
                .iter()
                .map(|tx| tx.signed_transaction().clone())
                .collect::<Vec<_>>();

            assert_eq!(signed_txns, vec![signed_tx1.clone(), signed_tx2.clone()]);
            assert_eq!(evm.pending_transactions(state).len(), 0);

            assert_eq!(evm.get_tx_index_by_hash(&signed_tx1.hash(), state), Some(0));
            assert_eq!(evm.get_tx_index_by_hash(&signed_tx2.hash(), state), Some(1));
        }),
    });
}
