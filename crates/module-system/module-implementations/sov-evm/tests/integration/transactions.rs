use crate::helpers::setup;
use crate::helpers::EvmAccount;
use crate::runtime::{RT, S};
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::Address;
use alloy_primitives::{Bytes, TxKind, U256};
use sov_evm::{EthereumAuthenticator, Evm};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::{BatchTestCase, SimpleStorageContract, TransactionType};

#[test]
fn test_executing_eth_transaction() {
    let (mut runner, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr = account.address().create(0);
    let create_contract_tx = create_contract_tx(0, &contract, &account);

    let set_value_tx = create_set_arg_tx(5, 1, &contract, contract_addr, &account);

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

    for n in 2..10 {
        let address = account.address();
        let set_value_tx =
            create_set_arg_tx((n + 90) as u32, n, &contract, contract_addr, &account);

        runner.execute_batch(BatchTestCase {
            input: vec![TransactionType::<RT, S>::PreAuthenticated(
                RT::encode_with_ethereum_auth(set_value_tx),
            )]
            .into(),
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
    let create_contract_tx = create_contract_tx(0, &contract, &no_balance_account);

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

fn create_contract_tx(nonce: u64, contract: &SimpleStorageContract, account: &EvmAccount) -> RawTx {
    let create_contract_tx_request = TypedTransaction::Eip1559(TxEip1559 {
        chain_id: config_value!("CHAIN_ID"),
        nonce,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        gas_limit: 1_000_000,
        to: TxKind::Create,
        value: Default::default(),
        input: Bytes::from(contract.byte_code().to_vec()),
        access_list: Default::default(),
    });
    let (signed_eth_tx, _) = account.sign(create_contract_tx_request);
    RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    }
}

fn create_set_arg_tx(
    set_arg: u32,
    nonce: u64,
    contract: &SimpleStorageContract,
    contract_addr: Address,
    account: &EvmAccount,
) -> RawTx {
    let set_arg_eth_tx = TypedTransaction::Eip1559(TxEip1559 {
        chain_id: config_value!("CHAIN_ID"),
        nonce,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        gas_limit: 1_000_000,
        to: TxKind::Call(contract_addr),
        value: Default::default(),
        input: Bytes::from(hex::decode(hex::encode(contract.set_call_data(set_arg))).unwrap()),
        access_list: Default::default(),
    });

    let (signed_eth_tx, _) = account.sign(set_arg_eth_tx);
    RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    }
}
