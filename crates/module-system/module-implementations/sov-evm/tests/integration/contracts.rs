use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{Bytes, TxKind};
use sov_evm::{EthereumAuthenticator, Evm};
use sov_evm_test_utils::LegacySimpleStorage;
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::{TransactionTestCase, TransactionType};

use crate::helpers::setup;
use crate::runtime::{RT, S};

#[test]
fn test_invalid_contract_execution() {
    let (mut runner, account, _, _) = setup();
    let contract = LegacySimpleStorage::default();
    let contract_addr = account.address().create(0);
    let tx_request = TypedTransaction::Eip1559(TxEip1559 {
        chain_id: config_value!("CHAIN_ID"),
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        gas_limit: 1_000_000,
        input: Bytes::from(contract.byte_code().to_vec()),
        ..Default::default()
    });
    let (signed_eth_tx, _) = account.sign(tx_request);
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };

    runner.execute(TransactionType::<RT, S>::PreAuthenticated(
        RT::encode_with_ethereum_auth(raw_tx),
    ));

    let tx_request = TypedTransaction::Eip1559(TxEip1559 {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 1,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        gas_limit: 1_000_000,
        to: TxKind::Call(contract_addr),
        input: Bytes::from(hex::decode(hex::encode(contract.failing_function())).unwrap()),
        ..Default::default()
    });
    let (signed_eth_tx, _) = account.sign(tx_request);
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };

    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
        assert: Box::new(|ctx, state| {
            assert!(ctx.tx_receipt.is_successful());

            let evm = Evm::<S>::default();
            let receipt = evm
                .receipt(1, state)
                .expect("failing contract call should have an EVM receipt");
            assert!(!receipt.0.receipt.success);
        }),
    });
}

#[test]
fn test_get_empty_code() {
    let (runner, account, _, _) = setup();
    let address_without_code = account.address();

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let code = evm.get_code(address_without_code, None, state).unwrap();
        assert_eq!(&code.to_string(), "0x");
    });
}
