use crate::helpers::{
    create_deploy_tx, create_set_arg_tx, create_transfer_tx, set_max_fee_check_height, setup,
    EvmAccount,
};
use crate::runtime::{GenesisConfig, RT, S};
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{Address, Bytes, TxKind, B256, U256};
use alloy_rpc_types::{
    state::{AccountOverride, StateOverride},
    BlockOverrides, TransactionRequest,
};
use jsonrpsee::types::error::INVALID_PARAMS_CODE;
use sov_address::{EthereumAddress, MultiAddress};
use sov_evm::{
    AccountData, ContractCreationPolicy, EthereumAuthenticator, Evm, EvmChainSpec,
    EvmGenesisConfig, SpecId,
};
use sov_evm_test_utils::LegacySimpleStorage;
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{TransactionType, TEST_DEFAULT_USER_BALANCE};
use std::collections::BTreeMap;

fn create_deploy_tx_with_init_code(
    nonce: u64,
    account: &EvmAccount,
    init_code: Bytes,
) -> TransactionType<RT, S> {
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
    TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx))
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

fn runtime_require_block_number(block_number: u8) -> Bytes {
    Bytes::from(vec![
        0x43,
        0x60,
        block_number,
        0x14,
        0x60,
        0x0c,
        0x57,
        0x60,
        0x00,
        0x60,
        0x00,
        0xfd,
        0x5b,
        0x00,
    ])
}

fn runtime_returning_constant(value: U256) -> Bytes {
    let mut runtime = vec![0x7f];
    runtime.extend_from_slice(&value.to_be_bytes::<32>());
    runtime.extend_from_slice(&[0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    Bytes::from(runtime)
}

fn runtime_returning_push0() -> Bytes {
    Bytes::from(vec![0x5f, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3])
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

fn call_request(from: Address, to: Address) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(to)),
        ..Default::default()
    }
}

fn setup_with_hardforks(hardforks: Vec<(u64, SpecId)>) -> (TestRunner<RT, S>, EvmAccount) {
    let evm_account = EvmAccount::generate();
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config
        .additional_accounts()
        .first()
        .expect("genesis should include a default account")
        .clone();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData::empty_with_address(evm_account.address())],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks,
        },
        contract_creation_policy: ContractCreationPolicy::Everyone,
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);
    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
    (runner, evm_account)
}

#[test]
fn test_eth_call_state_override_code_is_applied() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x42);
    let expected = U256::from(0x1234u64);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = call_request(account.address(), target);

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
        let request = call_request(account.address(), target);

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
fn test_eth_call_rejects_block_override_base_fee_overflow() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x54);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = call_request(account.address(), target);

        let err = evm
            .eth_call(
                request,
                None,
                None,
                Some(Box::new(BlockOverrides::default().with_base_fee(U256::MAX))),
                state,
            )
            .unwrap_err();
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
    });
}

#[test]
fn test_eth_call_rejects_block_override_number_overflow() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x5A);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = call_request(account.address(), target);

        let err = evm
            .eth_call(
                request,
                None,
                None,
                Some(Box::new(BlockOverrides::default().with_number(U256::MAX))),
                state,
            )
            .unwrap_err();
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
    });
}

#[test]
fn test_eth_call_rejects_move_precompile_override() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x5B);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = call_request(account.address(), target);

        let mut account_override = AccountOverride::default();
        account_override.set_move_precompile_to(Address::with_last_byte(0x11));
        let mut state_overrides = StateOverride::default();
        state_overrides.insert(target, account_override);

        let err = evm
            .eth_call(request, None, Some(state_overrides), None, state)
            .unwrap_err();
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
    });
}

#[test]
fn test_eth_estimate_gas_state_override_code_is_applied() {
    let (mut runner, account, _, _) = setup();
    let contract = LegacySimpleStorage::default();
    let contract_addr = account.address().create(0);
    runner.execute(create_deploy_tx(0, &contract, &account).tx);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = TransactionRequest {
            from: Some(account.address()),
            to: Some(TxKind::Call(contract_addr)),
            input: contract.always_revert().into(),
            ..Default::default()
        };

        assert!(evm
            .eth_estimate_gas_helper(request.clone(), None, None, None, state)
            .is_err());

        let mut state_overrides = StateOverride::default();
        state_overrides.insert(
            contract_addr,
            AccountOverride::default().with_code(Bytes::from(vec![0x00])),
        );
        let estimate = evm
            .eth_estimate_gas_helper(request, None, Some(state_overrides), None, state)
            .unwrap();
        assert!(estimate.to::<u64>() > 0);
    });
}

#[test]
fn test_eth_estimate_gas_block_override_number_is_applied() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x55);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = call_request(account.address(), target);

        let mut state_overrides = StateOverride::default();
        state_overrides.insert(
            target,
            AccountOverride::default().with_code(runtime_require_block_number(77)),
        );

        assert!(evm
            .eth_estimate_gas_helper(
                request.clone(),
                None,
                Some(state_overrides.clone()),
                None,
                state
            )
            .is_err());

        let estimate = evm
            .eth_estimate_gas_helper(
                request,
                None,
                Some(state_overrides),
                Some(Box::new(
                    BlockOverrides::default().with_number(U256::from(77u64)),
                )),
                state,
            )
            .unwrap();
        assert!(estimate.to::<u64>() > 0);
    });
}

#[test]
fn test_eth_call_and_estimate_gas_prefer_fee_cap_over_stale_nonce_after_block_override() {
    set_max_fee_check_height(0);

    let (mut runner, account, recipient, _) = setup();
    runner.execute(create_transfer_tx(0, &account, &recipient, 1).tx);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = TransactionRequest {
            from: Some(account.address()),
            to: Some(TxKind::Call(recipient.address())),
            gas: Some(21_000),
            nonce: Some(0),
            max_fee_per_gas: Some(1),
            max_priority_fee_per_gas: Some(0),
            ..Default::default()
        };
        let block_overrides = BlockOverrides::default().with_base_fee(U256::from(2u64));

        let call_err = evm
            .eth_call(
                request.clone(),
                None,
                None,
                Some(Box::new(block_overrides.clone())),
                state,
            )
            .unwrap_err();
        assert!(
            call_err
                .message()
                .contains("max fee per gas less than block base fee"),
            "unexpected eth_call error: {call_err}"
        );

        let estimate_err = evm
            .eth_estimate_gas_helper(request, None, None, Some(Box::new(block_overrides)), state)
            .unwrap_err();
        assert!(
            estimate_err
                .message()
                .contains("max fee per gas less than block base fee"),
            "unexpected eth_estimateGas error: {estimate_err}"
        );
    });
}

#[test]
fn test_omitted_gas_respects_lower_block_gas_limit_override() {
    let (runner, account, _, _) = setup();
    let target = Address::with_last_byte(0x56);
    let expected = U256::from(0x1234u64);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = call_request(account.address(), target);
        let mut state_overrides = StateOverride::default();
        state_overrides.insert(
            target,
            AccountOverride::default().with_code(runtime_returning_constant(expected)),
        );
        let block_overrides = BlockOverrides::default().with_gas_limit(500_000);

        let output = evm
            .eth_call(
                request.clone(),
                None,
                Some(state_overrides.clone()),
                Some(Box::new(block_overrides.clone())),
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(output), expected);

        let estimate = evm
            .eth_estimate_gas_helper(
                request,
                None,
                Some(state_overrides),
                Some(Box::new(block_overrides)),
                state,
            )
            .unwrap();
        assert!(estimate.to::<u64>() > 0);
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

    runner.execute(create_deploy_tx_with_init_code(
        0,
        &account,
        create_init_code(basefee_runtime.as_ref()),
    ));
    runner.execute(create_deploy_tx_with_init_code(
        1,
        &account,
        create_init_code(number_runtime.as_ref()),
    ));
    runner.execute(create_deploy_tx_with_init_code(
        2,
        &account,
        create_init_code(timestamp_runtime.as_ref()),
    ));

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let block_overrides = BlockOverrides::default()
            .with_base_fee(U256::from(1_234u64))
            .with_time(5_555)
            .with_number(U256::from(77u64));

        let basefee_output = evm
            .eth_call(
                call_request(account.address(), basefee_contract_addr),
                None,
                None,
                Some(Box::new(block_overrides.clone())),
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(basefee_output), U256::from(1_234u64));

        let number_output = evm
            .eth_call(
                call_request(account.address(), number_contract_addr),
                None,
                None,
                Some(Box::new(block_overrides.clone())),
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(number_output), U256::from(77u64));

        let timestamp_output = evm
            .eth_call(
                call_request(account.address(), timestamp_contract_addr),
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
fn test_eth_call_block_number_override_changes_hardfork_spec() {
    let (runner, account) = setup_with_hardforks(vec![(0, SpecId::LONDON), (100, SpecId::CANCUN)]);
    let push0_contract_addr = Address::with_last_byte(0x56);
    let number_contract_addr = Address::with_last_byte(0x57);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let mut push0_overrides = StateOverride::default();
        push0_overrides.insert(
            push0_contract_addr,
            AccountOverride::default().with_code(runtime_returning_push0()),
        );

        assert!(evm
            .eth_call(
                call_request(account.address(), push0_contract_addr),
                None,
                Some(push0_overrides.clone()),
                None,
                state,
            )
            .is_err());

        // Overriding block number to 100 activates CANCUN, so PUSH0 should now succeed.
        let block_overrides = BlockOverrides::default().with_number(U256::from(100u64));
        assert!(evm
            .eth_call(
                call_request(account.address(), push0_contract_addr),
                None,
                Some(push0_overrides),
                Some(Box::new(block_overrides.clone())),
                state,
            )
            .is_ok());

        let mut number_overrides = StateOverride::default();
        number_overrides.insert(
            number_contract_addr,
            AccountOverride::default().with_code(runtime_returning_opcode(0x43)),
        );
        let number_output = evm
            .eth_call(
                call_request(account.address(), number_contract_addr),
                None,
                Some(number_overrides),
                Some(Box::new(block_overrides)),
                state,
            )
            .unwrap();
        assert_eq!(decode_u256(number_output), U256::from(100u64));
    });
}

#[test]
fn test_eth_call_block_hash_override_is_applied() {
    let (mut runner, account, _, _) = setup();
    let contract_addr = account.address().create(0);
    let runtime = runtime_returning_blockhash(0);
    runner.execute(create_deploy_tx_with_init_code(
        0,
        &account,
        create_init_code(runtime.as_ref()),
    ));

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let request = call_request(account.address(), contract_addr);

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
