use std::str::FromStr;

use alloy_primitives::{Address, U256};
use sov_evm::{CallMessage, Evm, EvmRuntimeConfigUpdate};
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::TransactionType;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase, TEST_DEFAULT_USER_BALANCE};

use crate::helpers::{
    create_transfer_tx_with_fee_params, create_transfer_tx_with_max_fee, set_max_fee_check_height,
    setup,
};
use crate::runtime::{RT, S};

/// Test that EVM_MAX_FEE_CHECK_HEIGHT is respected:
/// - Before the height threshold, transactions with insufficient max_fee pass.
/// - After the height threshold, transactions with insufficient max_fee fail.
/// - After disabling the check via UpdateRuntimeConfig, transactions pass again.
#[test]
fn test_max_fee_check_height_is_respected() {
    // Set the max fee check to activate after block 3
    set_max_fee_check_height(3);

    let (mut runner, evm_account, _, admin) = setup();
    let to_address = Address::from_str("0x0123456789012345678901234567890123456789").unwrap();
    let mut nonce = 0u64;

    let very_low_fee = 1u128;

    // Block 1: Transaction with low fee should PASS.
    let low_fee_tx = create_transfer_tx_with_max_fee(nonce, &evm_account, to_address, very_low_fee);
    nonce += 1;
    execute_tx_expect_success(
        &mut runner,
        low_fee_tx,
        "Transaction should pass before height threshold",
    );

    runner.advance_slots(2);

    // Block 5: Transaction with low fee should FAIL.
    // Note: We don't increment nonce because this transaction will fail/be skipped
    let low_fee_tx = create_transfer_tx_with_max_fee(nonce, &evm_account, to_address, very_low_fee);
    execute_tx_expect_failure(
        &mut runner,
        low_fee_tx,
        "Transaction should fail after height threshold",
    );

    let high_fee_tx = create_transfer_tx_with_max_fee(nonce, &evm_account, to_address, 99);
    execute_tx_expect_success(&mut runner, high_fee_tx, "Transaction should succeed");
    nonce += 1;

    // Block 6: Disable the max fee check via empty config update
    disable_max_fee_check(&mut runner, &admin);

    // Block 7: Transaction with low fee should PASS.
    let low_fee_tx = create_transfer_tx_with_max_fee(nonce, &evm_account, to_address, very_low_fee);
    execute_tx_expect_success(
        &mut runner,
        low_fee_tx,
        "Transaction should pass after disabling max fee check",
    );
}

/// Test that disabling max fee validation does not create value in the EVM state:
/// sender+recipient balances must only decrease by the charged fee.
#[test]
fn test_disable_max_fee_check_does_not_mint_value() {
    set_max_fee_check_height(0);

    let (mut runner, from, to, admin) = setup();
    disable_max_fee_check(&mut runner, &admin);

    let tx = create_transfer_tx_with_fee_params(0, &from, &to, 1, 1, 0);
    let from_address = from.address();
    let to_address = to.address();
    let tx_hash = tx.hash;

    let evm = Evm::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: tx.tx,
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());

            let sender_balance_after = evm.get_balance(from_address, None, state).unwrap();
            let recipient_balance_after = evm.get_balance(to_address, None, state).unwrap();
            let initial_total = U256::from(TEST_DEFAULT_USER_BALANCE.0);
            let transfer_value = U256::from(1u64);

            assert_eq!(recipient_balance_after, transfer_value);

            let sender_plus_recipient = sender_balance_after
                .checked_add(recipient_balance_after)
                .expect("balance addition should not overflow");
            assert!(
                sender_plus_recipient <= initial_total,
                "sender+recipient balance should never exceed initial balance"
            );

            let actual_fee = initial_total
                .checked_sub(sender_plus_recipient)
                .expect("actual fee should be non-negative");
            assert!(
                actual_fee > U256::ZERO,
                "transaction should still pay a positive fee after disabling max fee check"
            );

            let receipt = evm
                .get_transaction_receipt(tx_hash, state)
                .unwrap()
                .expect("receipt should exist");
            let implied_fee =
                U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price);

            assert!(
                implied_fee >= actual_fee,
                "receipt-implied fee should not under-report the actual charged fee"
            );
            let over_reported_fee = implied_fee
                .checked_sub(actual_fee)
                .expect("implied fee is checked to be >= actual fee");
            if receipt.effective_gas_price == 0 {
                assert_eq!(
                    over_reported_fee,
                    U256::ZERO,
                    "zero effective gas price cannot over-report fee"
                );
            } else {
                assert!(
                    over_reported_fee < U256::from(receipt.effective_gas_price),
                    "receipt-implied fee overage should stay below one gas-price unit"
                );
            }
        }),
    });
}

/// Execute a transaction and assert it succeeds.
fn execute_tx_expect_success(
    runner: &mut TestRunner<RT, S>,
    input: TransactionType<RT, S>,
    error_msg: &'static str,
) {
    runner.execute_transaction(TransactionTestCase {
        input,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "{}: {:?}",
                error_msg,
                ctx.tx_receipt
            );
        }),
    });
}

/// Execute a transaction and assert it fails.
fn execute_tx_expect_failure(
    runner: &mut TestRunner<RT, S>,
    input: TransactionType<RT, S>,
    error_msg: &'static str,
) {
    runner.execute_transaction(TransactionTestCase {
        input,
        assert: Box::new(move |ctx, _state| {
            assert!(
                !ctx.tx_receipt.is_successful(),
                "{}: {:?}",
                error_msg,
                ctx.tx_receipt
            );
        }),
    });
}

/// Disable the max fee check by sending an empty EvmRuntimeConfigUpdate.
fn disable_max_fee_check(runner: &mut TestRunner<RT, S>, admin: &TestUser<S>) {
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate::empty(),
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            let evm = Evm::<S>::default();
            assert!(
                evm.is_max_fee_check_disabled(state).unwrap(),
                "disable_max_fee_check should be true after empty config update"
            );
        }),
    });
}
