use crate::helpers::{
    create_transfer_tx, set_max_fee_check_height, set_receipt_actual_fee_height, setup, EvmAccount,
};
use crate::runtime::{RT, S};
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::eip2930::{AccessList, AccessListItem};
use alloy_eips::BlockId;
use alloy_primitives::{Bytes, TxKind, B256, U256};
use alloy_rpc_types::TransactionRequest;
use sov_evm::{CallMessage, EthereumAuthenticator, Evm, EvmRuntimeConfigUpdate};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase, TransactionType};

const BASEFEE_CONTRACT_INIT_CODE_HEX: &str = "6a60004860005260206000f3600052600b6015f3";

struct TxWithHash {
    tx: TransactionType<RT, S>,
    hash: B256,
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
    let (signed_eth_tx, signed_tx) = account.sign(TypedTransaction::Eip1559(tx));
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).expect("RLP tx should serialize"),
    };
    TxWithHash {
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
        hash: *signed_tx.hash(),
    }
}

fn create_call_tx(
    nonce: u64,
    gas_limit: u64,
    account: &EvmAccount,
    to: alloy_primitives::Address,
    input: Bytes,
    access_list: AccessList,
) -> TxWithHash {
    let tx = TxEip1559 {
        to: TxKind::Call(to),
        input,
        access_list,
        nonce,
        gas_limit,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };
    let (signed_eth_tx, signed_tx) = account.sign(TypedTransaction::Eip1559(tx));
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).expect("RLP tx should serialize"),
    };
    TxWithHash {
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
        hash: *signed_tx.hash(),
    }
}

fn large_access_list(entry_count: usize) -> AccessList {
    AccessList(
        (0..entry_count)
            .map(|i| {
                let mut address = [0u8; 20];
                address[12..].copy_from_slice(&(i as u64).to_be_bytes());
                AccessListItem {
                    address: address.into(),
                    storage_keys: vec![],
                }
            })
            .collect(),
    )
}

fn estimate_gas_request(
    from: alloy_primitives::Address,
    to: alloy_primitives::Address,
) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(to)),
        max_fee_per_gas: Some(9),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    }
}

fn low_base_fee_override() -> Box<alloy_rpc_types::BlockOverrides> {
    Box::new(alloy_rpc_types::BlockOverrides::default().with_base_fee(U256::from(10u64)))
}

fn disable_max_fee_check(runner: &mut TestRunner<RT, S>, admin: &TestUser<S>) {
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate::empty(),
        )),
        assert: Box::new(|ctx, _state| {
            assert!(ctx.tx_receipt.is_successful());
        }),
    });
}

#[test]
fn test_eth_call_basefee_opcode_matches_block_header_base_fee() {
    let (mut runner, account, _, _) = setup();

    // init code returns this runtime:
    //   PUSH1 0x00
    //   BASEFEE
    //   PUSH1 0x00
    //   MSTORE
    //   PUSH1 0x20
    //   PUSH1 0x00
    //   RETURN
    let init_code = Bytes::from(
        hex::decode(BASEFEE_CONTRACT_INIT_CODE_HEX).expect("BASEFEE init code should be valid"),
    );
    let deploy_tx = create_deploy_tx_with_init_code(0, &account, init_code);
    let deploy_tx_hash = deploy_tx.hash;
    runner.execute(deploy_tx.tx);

    let caller = account.address();

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let receipt = evm
            .get_transaction_receipt(deploy_tx_hash, state)
            .unwrap()
            .expect("Deployment tx should exist");
        let contract_address = receipt
            .contract_address
            .expect("BASEFEE test contract deployment transaction failed");

        let deployed_code = evm
            .get_code(contract_address, Some(BlockId::number(1)), state)
            .unwrap();
        assert!(
            !deployed_code.is_empty(),
            "BASEFEE test contract was not deployed"
        );

        let block = evm
            .get_block_by_number(Some(BlockId::number(1)), Some(false), state)
            .unwrap()
            .expect("Block 1 should exist after executing one transaction");
        let expected_base_fee = block
            .header
            .base_fee_per_gas
            .expect("Sealed block should expose base fee");
        assert!(
            expected_base_fee > 0,
            "Test precondition failed: block base fee is zero"
        );

        let output = evm
            .eth_call(
                TransactionRequest {
                    from: Some(caller),
                    to: Some(TxKind::Call(contract_address)),
                    ..Default::default()
                },
                Some(BlockId::number(1)),
                None,
                None,
                state,
            )
            .unwrap();
        let observed_base_fee = U256::from_be_slice(output.as_ref());

        assert_eq!(
            observed_base_fee,
            U256::from(expected_base_fee),
            "eth_call BASEFEE opcode output does not match the block header base fee"
        );
    });
}

#[test]
fn test_eth_estimate_gas_skips_fee_cap_check_before_activation_height() {
    set_max_fee_check_height(100);

    let (runner, account, recipient, _) = setup();

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        // The request has max_fee_per_gas below the base fee. Before the
        // fee-check activation height the estimator must not reject it.
        evm.eth_estimate_gas_helper(
            estimate_gas_request(account.address(), recipient.address()),
            None,
            None,
            Some(low_base_fee_override()),
            state,
        )
        .expect("estimate should succeed when fee check is inactive");
    });
}

#[test]
fn test_eth_estimate_gas_skips_fee_cap_check_when_runtime_disabled() {
    set_max_fee_check_height(0);

    let (mut runner, account, recipient, admin) = setup();
    runner.execute(create_transfer_tx(0, &account, &recipient, 1).tx);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let err = evm
            .eth_estimate_gas_helper(
                estimate_gas_request(account.address(), recipient.address()),
                None,
                None,
                Some(low_base_fee_override()),
                state,
            )
            .unwrap_err();

        assert!(
            err.message()
                .contains("max fee per gas less than block base fee"),
            "unexpected error: {err}"
        );
    });

    disable_max_fee_check(&mut runner, &admin);

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        // After disabling the max-fee check at runtime, the same request should succeed.
        evm.eth_estimate_gas_helper(
            estimate_gas_request(account.address(), recipient.address()),
            None,
            None,
            Some(low_base_fee_override()),
            state,
        )
        .expect("estimate should succeed when fee check is runtime-disabled");
    });
}

#[test]
fn test_eth_estimate_gas_large_access_list_underestimates_executed_receipt() {
    const ACCESS_LIST_ENTRIES: usize = 512;
    const TARGET_GAS_LIMIT: u64 = 10_000_000;

    set_max_fee_check_height(0);
    set_receipt_actual_fee_height(0);

    let (mut runner, account, recipient, _) = setup();
    runner.execute(create_transfer_tx(0, &account, &recipient, 1).tx);

    let access_list = large_access_list(ACCESS_LIST_ENTRIES);
    let request = TransactionRequest {
        from: Some(account.address()),
        to: Some(TxKind::Call(recipient.address())),
        access_list: Some(access_list.clone()),
        max_fee_per_gas: Some((MIN_PROTOCOL_BASE_FEE as u128) * 2),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };

    let estimate = runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        evm.eth_estimate_gas_helper(request.clone(), None, None, None, state)
            .unwrap()
            .to::<u64>()
    });

    let tx = create_call_tx(
        1,
        TARGET_GAS_LIMIT,
        &account,
        recipient.address(),
        Bytes::default(),
        access_list,
    );
    let tx_hash = tx.hash;
    runner.execute_transaction(TransactionTestCase {
        input: tx.tx,
        assert: Box::new(|ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "large access list transaction should succeed: {:?}",
                ctx.tx_receipt
            );
        }),
    });

    runner.query_visible_state(move |state| {
        let evm = Evm::<S>::default();
        let receipt = evm
            .get_transaction_receipt(tx_hash, state)
            .unwrap()
            .expect("large access list transaction receipt should exist");

        assert!(
            receipt.gas_used > estimate,
            "large access list transaction should expose estimate under-reporting; estimate={estimate}, receipt_gas_used={}",
            receipt.gas_used
        );
    });
}

#[test]
fn test_eth_estimate_gas_historical_block_below_receipt_projection_height_skips_projection() {
    const EVM_RECEIPT_ACTUAL_FEE_HEIGHT: u64 = 3;
    const HISTORICAL_BLOCK: u64 = 1;

    set_max_fee_check_height(0);
    set_receipt_actual_fee_height(EVM_RECEIPT_ACTUAL_FEE_HEIGHT);

    let (mut runner, account, recipient, _) = setup();
    runner.execute(create_transfer_tx(0, &account, &recipient, 1).tx);
    runner.advance_slots((EVM_RECEIPT_ACTUAL_FEE_HEIGHT + 1) as usize);

    let request = TransactionRequest {
        from: Some(account.address()),
        to: Some(TxKind::Call(recipient.address())),
        max_fee_per_gas: Some((MIN_PROTOCOL_BASE_FEE as u128) * 10),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };

    let (historical_estimate, latest_estimate) = runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let live_block_number = evm.block_number(state).unwrap().to::<u64>();
        assert!(
            live_block_number > EVM_RECEIPT_ACTUAL_FEE_HEIGHT,
            "test precondition failed: live height {live_block_number} must be above EVM_RECEIPT_ACTUAL_FEE_HEIGHT={EVM_RECEIPT_ACTUAL_FEE_HEIGHT}"
        );

        let historical_estimate = evm
            .eth_estimate_gas_helper(
                request.clone(),
                Some(BlockId::number(HISTORICAL_BLOCK)),
                None,
                Some(low_base_fee_override()),
                state,
            )
            .unwrap()
            .to::<u64>();
        let latest_estimate = evm
            .eth_estimate_gas_helper(
                request,
                Some(BlockId::latest()),
                None,
                Some(low_base_fee_override()),
                state,
            )
            .unwrap()
            .to::<u64>();

        (historical_estimate, latest_estimate)
    });

    assert!(
        latest_estimate > historical_estimate,
        "historical eth_estimateGas for block {HISTORICAL_BLOCK}, which is below EVM_RECEIPT_ACTUAL_FEE_HEIGHT={EVM_RECEIPT_ACTUAL_FEE_HEIGHT}, should skip fee projection while latest uses it; historical_estimate={historical_estimate}, latest_estimate={latest_estimate}"
    );
}

#[test]
fn test_eth_estimate_gas_historical_block_below_fee_check_height_keeps_same_estimate() {
    const EVM_MAX_FEE_CHECK_HEIGHT: u64 = 3;
    const HISTORICAL_BLOCK: u64 = 1;

    set_max_fee_check_height(EVM_MAX_FEE_CHECK_HEIGHT);

    let (mut runner, account, recipient, _) = setup();
    runner.execute(create_transfer_tx(0, &account, &recipient, 1).tx);

    let request = TransactionRequest {
        from: Some(account.address()),
        to: Some(TxKind::Call(recipient.address())),
        // Keep the request admissible in both regimes so only the multiplier source matters.
        max_fee_per_gas: Some((MIN_PROTOCOL_BASE_FEE as u128) * 10),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };

    let estimate_before_threshold = runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let live_block_number = evm.block_number(state).unwrap().to::<u64>();
        assert!(
            live_block_number <= EVM_MAX_FEE_CHECK_HEIGHT,
            "test precondition failed: live height {live_block_number} must still be at or below EVM_MAX_FEE_CHECK_HEIGHT={EVM_MAX_FEE_CHECK_HEIGHT}"
        );

        evm.eth_estimate_gas_helper(
            request.clone(),
            Some(BlockId::number(HISTORICAL_BLOCK)),
            None,
            None,
            state,
        )
        .unwrap()
        .to::<u64>()
    });

    runner.advance_slots((EVM_MAX_FEE_CHECK_HEIGHT + 1) as usize);

    let estimate_after_threshold = runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let live_block_number = evm.block_number(state).unwrap().to::<u64>();
        assert!(
            live_block_number > EVM_MAX_FEE_CHECK_HEIGHT,
            "test precondition failed: live height {live_block_number} must be above EVM_MAX_FEE_CHECK_HEIGHT={EVM_MAX_FEE_CHECK_HEIGHT}"
        );

        evm.eth_estimate_gas_helper(
            request.clone(),
            Some(BlockId::number(HISTORICAL_BLOCK)),
            None,
            None,
            state,
        )
        .unwrap()
        .to::<u64>()
    });

    assert_eq!(
        estimate_before_threshold, estimate_after_threshold,
        "historical eth_estimateGas for block {HISTORICAL_BLOCK}, which is below EVM_MAX_FEE_CHECK_HEIGHT={EVM_MAX_FEE_CHECK_HEIGHT}, should not change after the live chain crosses the activation height"
    );
}
