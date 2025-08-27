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
    let (mut runner, _, account, _) = setup();
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
    let (mut runner, _, _, no_balance_account) = setup();
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

fn create_deploy_tx(
    nonce: u64,
    contract: &SimpleStorageContract,
    account: &EvmAccount,
) -> TransactionType<RT, S> {
    let tx = TxEip1559 {
        input: Bytes::from(contract.byte_code().to_vec()),
        nonce,
        ..Default::default()
    };
    create_tx(account, tx)
}

fn create_set_arg_tx(
    set_arg: u32,
    nonce: u64,
    contract: &SimpleStorageContract,
    contract_addr: Address,
    account: &EvmAccount,
) -> TransactionType<RT, S> {
    let tx = TxEip1559 {
        to: TxKind::Call(contract_addr),
        input: Bytes::from(hex::decode(hex::encode(contract.set_call_data(set_arg))).unwrap()),
        nonce,
        ..Default::default()
    };
    create_tx(account, tx)
}

fn create_tx(account: &EvmAccount, tx: TxEip1559) -> TransactionType<RT, S> {
    let tx_with_defaults = TxEip1559 {
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..tx
    };
    let (signed_eth_tx, _) = account.sign(TypedTransaction::Eip1559(tx_with_defaults));
    let data = borsh::to_vec(&signed_eth_tx).unwrap();
    let raw_tx = RawTx { data };
    TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx))
}
