use sov_bank::{config_gas_token_id, Bank, Coins, ReserveGasError};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{Amount, Gas, GasUnit, SkippedTxContents, Spec, TxEffect};
use sov_test_utils::{get_gas_used, AsUser, TestUser, TransactionTestCase, TxProcessingError};

use crate::helpers::{setup, TestData, RT};

type S = sov_test_utils::TestSpec;

/// Tests the happy path of the `reserve_gas` method. We send a transfer transaction and check that the user balance is
/// correctly updated and reflects the amount of gas consumed and not the maximum fee.
/// The priority fee is zero.
#[test]
fn test_honest_reserve_gas_capability_without_priority_fee() {
    let (
        TestData {
            token_id,
            user_high_token_balance: sender,
            user_no_token_balance: receiver,
            ..
        },
        mut runner,
    ) = setup();

    const TRANSFER_AMOUNT: u128 = 10;
    let receiver_address = receiver.address();
    let sender_balance = sender.available_gas_balance;
    let max_fee = sender_balance;
    let sender_address = sender.address();

    runner.execute_transaction(TransactionTestCase {
        input: sender
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
                to: receiver_address,
                coins: Coins {
                    token_id,
                    amount: TRANSFER_AMOUNT.into(),
                },
            })
            .with_max_fee(max_fee)
            .with_max_priority_fee_bips(PriorityFeeBips::ZERO),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&sender_address, config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(sender_balance.checked_sub(result.gas_value_used).unwrap())
            );

            assert!( Amount::ZERO < result.gas_value_used && result.gas_value_used < sender_balance, "The gas used should be positive and less than the sender balance, which is the max fee amount");
        }),
    });
}

/// Tests the happy path of the `reserve_gas` method. We try to execute a transfer transaction and check that the user
/// balance is correctly updated from the hooks.
/// The priority fee is non zero but the difference between the max fee and the maximum gas value is zero
/// hence the priority fee is not charged.
/// We simulate the transaction execution to get the amount of gas that should be consumed.
#[test]
fn test_honest_reserve_gas_capability_does_not_charge_priority_fee() {
    let (
        TestData {
            token_id,
            user_high_token_balance: sender,
            user_no_token_balance: receiver,
            ..
        },
        mut runner,
    ) = setup();

    const TRANSFER_AMOUNT: u128 = 10;
    const PRIORITY_FEE: PriorityFeeBips = PriorityFeeBips::from_percentage(10);
    let receiver_address = receiver.address();
    let sender_balance = sender.available_gas_balance;

    // We simulate the transaction execution to get the amount of gas that should be consumed by the transaction.
    let (simulation_result, _, _) = runner.simulate(
        sender
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
                to: receiver_address,
                coins: Coins {
                    token_id,
                    amount: TRANSFER_AMOUNT.into(),
                },
            })
            .with_max_fee(sender_balance)
            .with_max_priority_fee_bips(PriorityFeeBips::ZERO),
    );

    // From the simulation result we can compute the gas used value.
    let gas_used_simulation = get_gas_used(
        simulation_result
            .batch_receipts
            .last()
            .unwrap()
            .tx_receipts
            .last()
            .unwrap(),
    );

    let gas_price_simulation = <<S as Spec>::Gas as Gas>::Price::try_from(
        simulation_result
            .batch_receipts
            .last()
            .unwrap()
            .inner
            .gas_price
            .clone(),
    )
    .unwrap();

    let gas_used_value_simulation = gas_used_simulation.value(&gas_price_simulation);

    // Since the max fee is exactly the gas used by the transaction following the simulation, we expect the priority fee *not* to be charged.
    runner.execute_transaction(TransactionTestCase {
        input: sender
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
                to: receiver.address(),
                coins: Coins {
                    token_id,
                    amount: TRANSFER_AMOUNT.into(),
                },
            })
            .with_max_fee(gas_used_value_simulation)
            .with_max_priority_fee_bips(PRIORITY_FEE),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&sender.address(), config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    sender
                        .available_gas_balance
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                )
            );

            assert!(
                Amount::ZERO < result.gas_value_used
                    && result.gas_value_used == gas_used_value_simulation,
                "The gas used should be positive and exactly the max fee amount."
            );
        }),
    });
}

/// Tests the happy path of the `reserve_gas` method. We try to reserve gas, then consume it and refund it.
/// The priority fee is non zero and is charged as part of the transaction.
/// We simulate the transaction execution to get the amount of gas that should be consumed.
#[test]
fn test_honest_reserve_gas_capability_with_priority_fee() {
    let (
        TestData {
            token_id,
            user_high_token_balance: sender,
            user_no_token_balance: receiver,
            ..
        },
        mut runner,
    ) = setup();

    const TRANSFER_AMOUNT: u128 = 10;
    const PRIORITY_FEE: PriorityFeeBips = PriorityFeeBips::from_percentage(10);
    let sender_balance = sender.available_gas_balance;

    // We use a higher max fee to ensure that the priority fee is charged.
    runner.execute_transaction(TransactionTestCase {
        input: sender
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
                to: receiver.address(),
                coins: Coins {
                    token_id,
                    amount: TRANSFER_AMOUNT.into(),
                },
            })
            .with_max_fee(sender_balance)
            .with_max_priority_fee_bips(PRIORITY_FEE),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&sender.address(), config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    sender.available_gas_balance
                        .checked_sub(result.gas_value_used).unwrap()
                        .checked_sub(PRIORITY_FEE.apply(result.gas_value_used).unwrap()).unwrap()
                )
            );

            assert!(
                Amount::ZERO < result.gas_value_used && result.gas_value_used < sender_balance,
                "The gas used should be positive and lower than the sender balance, which is the max fee amount"
            );
        }),
    });
}

/// Tests that the `reserve_gas` method fails if the sender does not have a bank account for the gas token
#[test]
fn test_reserve_gas_no_account() {
    let (
        TestData {
            token_id,
            token_name,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_no_account = TestUser::<S>::generate(Amount::ZERO);
    let user_high_token_initial_balance =
        user_high_token_balance.token_balance(&token_name).unwrap();

    const TRANSFER_AMOUNT: Amount = Amount::new(10);

    // We transfer to the user without an account.
    runner.execute(user_high_token_balance.create_plain_message::<RT, Bank<S>>(
        sov_bank::CallMessage::Transfer {
            to: user_no_account.address(),
            coins: Coins {
                token_id,
                amount: TRANSFER_AMOUNT,
            },
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: user_no_account.create_plain_message::<RT, Bank<S>>(
            sov_bank::CallMessage::Transfer {
                to: user_high_token_balance.address(),
                coins: Coins {
                    token_id,
                    amount: TRANSFER_AMOUNT,
                },
            },
        ),
        assert: Box::new(move |result, state| {
            if let TxEffect::Skipped(SkippedTxContents {
                gas_used: _,
                error: TxProcessingError::CannotReserveGas(reason),
            }) = result.tx_receipt
            {
                assert_eq!(
                    reason,
                    ReserveGasError::AccountDoesNotExist {
                        account: user_no_account.address().to_string(),
                    }
                    .to_string(),
                    "The inner reserve gas error is incorrect"
                );
            } else {
                panic!("The transaction should have reverted with a skipped error");
            };

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_account.address(), config_gas_token_id(), state)
                    .unwrap_infallible(),
                None,
                "The user should not have any balance in the gas token"
            );

            // The user should still have the initial custom token balance.
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_account.address(), token_id, state)
                    .unwrap_infallible(),
                Some(TRANSFER_AMOUNT)
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_high_token_balance.address(), token_id, state)
                    .unwrap_infallible(),
                Some(
                    user_high_token_initial_balance
                        .checked_sub(TRANSFER_AMOUNT)
                        .unwrap()
                )
            );
        }),
    });
}

/// Tests that the `reserve_gas` method fails if the sender balance is not high enough to pay for the gas.
#[test]
fn test_reserve_gas_not_enough_balance() {
    let (
        TestData {
            token_id,
            user_high_token_balance: sender,
            user_no_token_balance: receiver,
            ..
        },
        mut runner,
    ) = setup();

    const TRANSFER_AMOUNT: u128 = 10;

    runner.execute_transaction(TransactionTestCase {
        input: sender
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
                to: receiver.address(),
                coins: Coins {
                    token_id,
                    amount: TRANSFER_AMOUNT.into(),
                },
            })
            .with_max_fee(Amount::MAX),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Skipped(SkippedTxContents {
                gas_used: _,
                error: TxProcessingError::CannotReserveGas(reason),
            }) = result.tx_receipt
            {
                assert_eq!(
                    reason,
                    ReserveGasError::InsufficientBalanceToReserveGas.to_string(),
                    "The inner reserve gas error is incorrect"
                );
            } else {
                panic!("The transaction should have reverted with a skipped error");
            };
        }),
    });
}

/// Tests that the `reserve_gas` method fails if the current gas price is too high to cover the maximum fee for the transaction.
/// This check is only performed if the `gas_limit` is set.
#[test]
fn test_reserve_gas_price_too_high() {
    let (
        TestData {
            token_id,
            user_high_token_balance: sender,
            user_no_token_balance: receiver,
            ..
        },
        mut runner,
    ) = setup();

    const TRANSFER_AMOUNT: u128 = 10;

    let sender_balance = sender.available_gas_balance;
    let sender_balance_u64: u64 = sender_balance.0
        .try_into()
        .expect("This test relies on setting the gas limit to half of the sender balance, but gas is only a u64. Lower the sender balance or update the test.");

    // We set the gas limit to the sender balance to ensure that the initial gas price is too high.
    runner.execute_transaction(TransactionTestCase {
        input: sender
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
                to: receiver.address(),
                coins: Coins {
                    token_id,
                    amount: TRANSFER_AMOUNT.into(),
                },
            })
            .with_max_fee(sender_balance)
            .with_gas_limit(Some(GasUnit::from([sender_balance_u64 / 2; 2]))),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Skipped(SkippedTxContents {
                gas_used: _,
                error: TxProcessingError::CannotReserveGas(reason),
            }) = result.tx_receipt
            {
                assert_eq!(
                    reason,
                    ReserveGasError::CurrentGasPriceTooHigh.to_string(),
                    "The inner reserve gas error is incorrect"
                );
            } else {
                panic!("The transaction should have reverted with a skipped error");
            };
        }),
    });
}
