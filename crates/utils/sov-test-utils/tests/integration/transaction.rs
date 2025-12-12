use sov_bank::{config_gas_token_id, Bank, Coins};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::{PriorityFeeBips, TxDetails};
use sov_modules_api::{Amount, GasUnit};
use sov_test_utils::runtime::TestOptimisticRuntimeCall;
use sov_test_utils::{
    assert_matches, AsUser, BatchTestCase, TestUser, TransactionTestCase, TransactionType,
    TxProcessingError, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE,
    TEST_DEFAULT_USER_BALANCE,
};
use sov_value_setter::ValueSetter;

use crate::helpers::{setup, RT, S};

/// Checks that the chain id of a transaction can be overridden.
#[test]
fn test_custom_transaction_details_chain_id() {
    let (admin, mut runner) = setup();

    let real_chain_id = config_value!("CHAIN_ID");
    let fake_chain_id = real_chain_id + 1;

    runner.execute_batch(BatchTestCase {
        input: vec![admin
            .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue {
                value: 1,
                gas: None,
            })
            .with_chain_id(fake_chain_id)]
        .into(),
        assert: Box::new(move |result, _state| {
            let batch_receipt = result.batch_receipt.as_ref().unwrap();
            let tx_receipts = &batch_receipt.tx_receipts;

            assert_eq!(tx_receipts.len(), 1);

            match &tx_receipts[0].receipt {
                sov_modules_api::TxEffect::Skipped(skipped) => {
                    assert_matches!(skipped.error, TxProcessingError::AuthenticationFailed(_));
                }

                unexpected => panic!("Expected TxEffect::Skipped but got {unexpected:?}"),
            }

            assert!(batch_receipt.inner.outcome.rewards.accumulated_penalty > 0);
        }),
    });
}

/// Checks that the max fee of a transaction can be overridden.
#[test]
fn test_custom_transaction_details_max_fee() {
    let (admin, mut runner) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue{value:10,gas: None})
            .with_max_fee(Amount::ZERO),
        assert: Box::new(move |result, _state| {
           match &result.tx_receipt {
                sov_modules_api::TxEffect::Skipped(skipped) => {
                    if let TxProcessingError::OutOfGas(error_message) = &skipped.error {
                        assert!(
                            error_message.contains("The amount to charge is greater than the funds available in the meter."),
                            "Error message doesn't contain with the expected phrase. Got: {error_message}"
                        );
                    } else {
                        panic!("Expected CannotReserveGas error, but got a different SkippedReason: {:?}", skipped.error);
                    }
                },
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };
        }),
    });
}

/// Checks that the priority fee of a transaction can be overridden and that this has the expected effect on the balance of the sender.
#[test]
fn test_custom_transaction_details_priority_fee_bips() {
    let (admin, mut runner) = setup();

    let max_fee = admin.available_gas_balance;
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue{value:10, gas: None})
            .with_max_fee(max_fee)
            .with_max_priority_fee_bips(priority_fee_bips),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&admin.address(), config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    admin.available_gas_balance
                        .checked_sub(result.gas_value_used).unwrap()
                        .checked_sub(priority_fee_bips.apply(result.gas_value_used).unwrap()).unwrap()),
                "The admin's balance should be equal to the initial balance minus the gas used to send the transaction and the priority fee"
            );

        }),
    });
}

/// Checks that the chain id of a transaction can be overridden.
#[test]
fn test_custom_transaction_details_gas_limit() {
    let (admin, mut runner) = setup();
    let available_gas_balance: u64 = admin.available_gas_balance.0.try_into().unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue{value: 10, gas: None})
            .with_max_fee(admin.available_gas_balance)
            // We set gas limit to have the maximum value for minimum possible price, which is 1.
            // This way any gas price above 1 will give `gas price too high` error.
            .with_gas_limit(Some(GasUnit::from([available_gas_balance; 2]))),
        assert: Box::new(move |result, _state| {
           match &result.tx_receipt {
                sov_modules_api::TxEffect::Skipped(skipped) => {
                    if let TxProcessingError::CannotReserveGas(error_message) = &skipped.error {
                        assert!(
                            error_message.contains("The current gas price is too high to cover the maximum fee for the transaction"),
                            "Error message doesn't contain with the expected phrase. Got: {error_message}"
                        );
                    } else {
                        panic!("Expected CannotReserveGas error, but got a different SkippedReason: {:?}", skipped.error);
                    }
                },
                unexpected => panic!("Expected transaction to revert, but got: {unexpected:?}"),
            };
        }),
    });
}

/// Tests that sending a transaction with the default details works and that the balance of the sender is updated correctly.
#[test]
fn test_default_transaction_details_works() {
    let (admin, mut runner) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: admin
            .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue{value:10,gas: None}),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&admin.address(), config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(admin.available_gas_balance.checked_sub(result.gas_value_used).unwrap()),
                "The admin's balance should be equal to the initial balance minus the gas used to send the transaction"
            );
        }),
    });
}

/// Checks the default transaction details format.
#[test]
fn test_default_transaction_details() {
    let user = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    let message = user.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
        to: user.address(),
        coins: Coins {
            amount: Amount::new(1000),
            token_id: config_gas_token_id(),
        },
    });

    match message {
        TransactionType::Plain {
            details,
            message,
            key,
        } => {
            assert_eq!(
                message,
                TestOptimisticRuntimeCall::Bank(sov_bank::CallMessage::Transfer {
                    to: user.address(),
                    coins: Coins {
                        amount: Amount::new(1000),
                        token_id: config_gas_token_id(),
                    },
                })
            );

            assert_eq!(key.as_hex(), user.private_key().as_hex());

            assert_eq!(details.max_priority_fee_bips, TEST_DEFAULT_MAX_PRIORITY_FEE);
            assert_eq!(details.max_fee, TEST_DEFAULT_MAX_FEE);
            assert_eq!(details.gas_limit, None);

            assert_eq!(details.chain_id, 4321);
        }
        _ => panic!("The message is not a plain message"),
    }
}

/// Tests that the transaction is correctly formatted
#[test]
fn test_custom_transaction_format() {
    let user = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    let message = user
        .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
            to: user.address(),
            coins: Coins {
                amount: Amount::new(1000),
                token_id: config_gas_token_id(),
            },
        })
        .with_max_fee(Amount::new(100))
        .with_max_priority_fee_bips(PriorityFeeBips::from_percentage(10))
        .with_gas_limit(Some(GasUnit::from([5; 2])))
        .with_chain_id(5555);

    match message {
        TransactionType::Plain {
            details,
            message,
            key,
        } => {
            assert_eq!(
                message,
                TestOptimisticRuntimeCall::Bank(sov_bank::CallMessage::Transfer {
                    to: user.address(),
                    coins: Coins {
                        amount: Amount::new(1000),
                        token_id: config_gas_token_id(),
                    },
                })
            );

            assert_eq!(key.as_hex(), user.private_key().as_hex());

            assert_eq!(
                details.max_priority_fee_bips,
                PriorityFeeBips::from_percentage(10)
            );
            assert_eq!(details.max_fee, 100);
            assert_eq!(details.gas_limit, Some(GasUnit::from([5; 2])));

            assert_eq!(details.chain_id, 5555);
        }
        _ => panic!("The message is not a plain message"),
    }
}

#[test]
fn test_custom_transaction_format_2() {
    let user = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    let message = user
        .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Transfer {
            to: user.address(),
            coins: Coins {
                amount: Amount::new(1000),
                token_id: config_gas_token_id(),
            },
        })
        .with_details(TxDetails {
            max_fee: Amount::new(100),
            max_priority_fee_bips: PriorityFeeBips::from_percentage(10),
            gas_limit: Some(GasUnit::from([5; 2])),
            chain_id: 5555,
        });

    match message {
        TransactionType::Plain {
            details,
            message,
            key,
        } => {
            assert_eq!(
                message,
                TestOptimisticRuntimeCall::Bank(sov_bank::CallMessage::Transfer {
                    to: user.address(),
                    coins: Coins {
                        amount: Amount::new(1000),
                        token_id: config_gas_token_id(),
                    },
                })
            );

            assert_eq!(key.as_hex(), user.private_key().as_hex());

            assert_eq!(
                details.max_priority_fee_bips,
                PriorityFeeBips::from_percentage(10)
            );
            assert_eq!(details.max_fee, 100);
            assert_eq!(details.gas_limit, Some(GasUnit::from([5; 2])));

            assert_eq!(details.chain_id, 5555);
        }
        _ => panic!("The message is not a plain message"),
    }
}
