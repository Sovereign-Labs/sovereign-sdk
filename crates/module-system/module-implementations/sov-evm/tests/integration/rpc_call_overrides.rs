use crate::helpers::{create_deploy_tx, create_set_arg_tx, setup, EvmAccount};
use crate::runtime::{RT, S};
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use alloy_rpc_types::{
    state::{AccountOverride, StateOverride},
    BlockOverrides, TransactionRequest,
};
use jsonrpsee::types::error::INVALID_PARAMS_CODE;
use sov_evm::{EthereumAuthenticator, Evm};
use sov_evm_test_utils::LegacySimpleStorage;
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::TransactionType;
use std::collections::BTreeMap;

struct TxWithHash {
    tx: TransactionType<RT, S>,
}

fn create_deploy_tx_with_init_code(
    nonce: u64,
    account: &EvmAccount,
    init_code: Bytes,
) -> TxWithHash {
    let tx = TxEip1559 {
        input: init_code,
        nonce,
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };
    let (signed_eth_tx, _) = account.sign(TypedTransaction::Eip1559(tx));
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).expect("RLP tx should serialize"),
    };
    TxWithHash {
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
    }
}

fn create_init_code(runtime_code: &[u8]) -> Bytes {
    assert!(runtime_code.len() <= u8::MAX as usize);
    let runtime_len = runtime_code.len() as u8;
    let runtime_offset = 12u8;

    let mut init_code = vec![
        0x60,
        runtime_len,
        0x60,
        runtime_offset,
        0x60,
        0x00,
        0x39,
        0x60,
        runtime_len,
        0x60,
        0x00,
        0xf3,
    ];
    init_code.extend_from_slice(runtime_code);
    Bytes::from(init_code)
}

fn runtime_returning_opcode(opcode: u8) -> Bytes {
    Bytes::from(vec![opcode, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3])
}

fn runtime_returning_blockhash(block_number: u8) -> Bytes {
    Bytes::from(vec![
        0x60,
        block_number,
        0x40,
        0x60,
        0x00,
        0x52,
        0x60,
        0x20,
        0x60,
        0x00,
        0xf3,
    ])
}

fn runtime_returning_constant(value: U256) -> Bytes {
    let mut runtime = vec![0x7f];
    runtime.extend_from_slice(&value.to_be_bytes::<32>());
    runtime.extend_from_slice(&[0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    Bytes::from(runtime)
}

fn decode_u256(output: Bytes) -> U256 {
    U256::from_be_slice(output.as_ref())
}

fn decode_b256(output: Bytes) -> B256 {
    B256::from_slice(output.as_ref())
}

fn to_b256(value: U256) -> B256 {
    B256::from(value.to_be_bytes::<32>())
}

#[test]
fn test_eth_call_state_override_code_is_applied() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x42);
    let expected = U256::from(0x1234u64);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = TransactionRequest {
            from: Some(account.address()),
            to: Some(TxKind::Call(target)),
            ..Default::default()
        };

        let baseline = evm
            .eth_call(request.clone(), None, None, None, state)
            .unwrap();
        assert!(baseline.is_empty(), "EOA call should return empty output");

        let mut state_overrides = StateOverride::default();
        state_overrides.insert(
            target,
            AccountOverride::default().with_code(runtime_returning_constant(expected)),
        );

        let output = evm
            .eth_call(request, None, Some(state_overrides), None, state)
            .unwrap();
        assert_eq!(decode_u256(output), expected);
    });
}

#[test]
fn test_eth_call_state_and_state_diff_overrides_update_storage() {
    let (mut runner, account, _, _) = setup();
    let contract = LegacySimpleStorage::default();
    let contract_addr = account.address().create(0);

    runner.execute(create_deploy_tx(0, &contract, &account).tx);
    runner.execute(create_set_arg_tx(5, 1, &contract, contract_addr, &account).tx);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = TransactionRequest {
            from: Some(account.address()),
            to: Some(TxKind::Call(contract_addr)),
            input: contract.get().into(),
            ..Default::default()
        };

        let baseline = evm
            .eth_call(request.clone(), None, None, None, state)
            .unwrap();
        assert_eq!(decode_u256(baseline), U256::from(5u64));

        let mut state_diff_overrides = StateOverride::default();
        state_diff_overrides.insert(
            contract_addr,
            AccountOverride::default().with_state_diff([(B256::ZERO, to_b256(U256::from(7u64)))]),
        );
        let output_with_state_diff = evm
            .eth_call(
                request.clone(),
                None,
                Some(state_diff_overrides),
                None,
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(output_with_state_diff), U256::from(7u64));

        let mut state_overrides = StateOverride::default();
        state_overrides.insert(
            contract_addr,
            AccountOverride::default().with_state([(B256::ZERO, to_b256(U256::from(9u64)))]),
        );
        let output_with_state = evm
            .eth_call(request, None, Some(state_overrides), None, state)
            .unwrap();
        assert_eq!(decode_u256(output_with_state), U256::from(9u64));
    });
}

#[test]
fn test_eth_call_rejects_state_and_state_diff_on_same_account() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x53);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = TransactionRequest {
            from: Some(account.address()),
            to: Some(TxKind::Call(target)),
            ..Default::default()
        };

        let mut state_overrides = StateOverride::default();
        state_overrides.insert(
            target,
            AccountOverride::default()
                .with_state([(B256::ZERO, B256::ZERO)])
                .with_state_diff([(B256::ZERO, B256::ZERO)]),
        );

        let err = evm
            .eth_call(request, None, Some(state_overrides), None, state)
            .unwrap_err();
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
    });
}

#[test]
fn test_eth_call_block_overrides_base_fee_and_number_are_applied() {
    let (mut runner, account, _, _) = setup();
    let basefee_contract_addr = account.address().create(0);
    let number_contract_addr = account.address().create(1);
    let timestamp_contract_addr = account.address().create(2);

    let basefee_runtime = runtime_returning_opcode(0x48);
    let number_runtime = runtime_returning_opcode(0x43);
    let timestamp_runtime = runtime_returning_opcode(0x42);

    runner.execute(
        create_deploy_tx_with_init_code(0, &account, create_init_code(basefee_runtime.as_ref())).tx,
    );
    runner.execute(
        create_deploy_tx_with_init_code(1, &account, create_init_code(number_runtime.as_ref())).tx,
    );
    runner.execute(
        create_deploy_tx_with_init_code(2, &account, create_init_code(timestamp_runtime.as_ref()))
            .tx,
    );

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let block_overrides = BlockOverrides::default()
            .with_base_fee(U256::from(1_234u64))
            .with_time(5_555)
            .with_number(U256::from(77u64));

        let basefee_output = evm
            .eth_call(
                TransactionRequest {
                    from: Some(account.address()),
                    to: Some(TxKind::Call(basefee_contract_addr)),
                    ..Default::default()
                },
                None,
                None,
                Some(Box::new(block_overrides.clone())),
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(basefee_output), U256::from(1_234u64));

        let number_output = evm
            .eth_call(
                TransactionRequest {
                    from: Some(account.address()),
                    to: Some(TxKind::Call(number_contract_addr)),
                    ..Default::default()
                },
                None,
                None,
                Some(Box::new(block_overrides.clone())),
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(number_output), U256::from(77u64));

        let timestamp_output = evm
            .eth_call(
                TransactionRequest {
                    from: Some(account.address()),
                    to: Some(TxKind::Call(timestamp_contract_addr)),
                    ..Default::default()
                },
                None,
                None,
                Some(Box::new(block_overrides)),
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(timestamp_output), U256::from(5_555u64));
    });
}

#[test]
fn test_eth_call_block_hash_override_is_applied() {
    let (mut runner, account, _, _) = setup();
    let contract_addr = account.address().create(0);
    let runtime = runtime_returning_blockhash(0);
    runner.execute(
        create_deploy_tx_with_init_code(0, &account, create_init_code(runtime.as_ref())).tx,
    );

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = TransactionRequest {
            from: Some(account.address()),
            to: Some(TxKind::Call(contract_addr)),
            ..Default::default()
        };

        let baseline = evm
            .eth_call(request.clone(), None, None, None, state)
            .unwrap();
        let baseline_hash = decode_b256(baseline);

        let custom_hash = B256::with_last_byte(0xAB);
        let block_overrides = BlockOverrides {
            block_hash: Some(BTreeMap::from([(0, custom_hash)])),
            ..Default::default()
        };
        let overridden = evm
            .eth_call(request, None, None, Some(Box::new(block_overrides)), state)
            .unwrap();
        let overridden_hash = decode_b256(overridden);

        assert_eq!(overridden_hash, custom_hash);
        assert_ne!(baseline_hash, overridden_hash);
    });
}
