use sov_bank::event::Event;
use sov_bank::utils::TokenHolder;
use sov_bank::{config_gas_token_id, Bank, Coins};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, Error, TxEffect};
use sov_test_utils::runtime::genesis::TestTokenName;
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::helpers::{setup, TestBankRuntimeEvent, TestData, RT};

type S = sov_test_utils::TestSpec;

/// We can burn tokens that are deployed on the bank.
#[test]
fn burn_deployed_tokens_happy_path() {
    let (
        TestData {
            token_name,
            token_id,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_token_balance = user_high_token_balance.token_balance(&token_name).unwrap();

    let user_address = user_high_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: user_token_balance,
                    token_id,
                },
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                TestBankRuntimeEvent::Bank(Event::TokenBurned {
                    owner: TokenHolder::User(user_address),
                    coins: Coins {
                        amount: user_token_balance,
                        token_id
                    }
                }),
                result.events[0]
            );

            // Check that the user's balance is now zero
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_address, token_id, state)
                    .unwrap_infallible(),
                Some(Amount::ZERO),
                "The user's balance should be zero"
            );
        }),
    });
}

#[test]
fn burn_deployed_tokens_no_balance_fails() {
    let (
        TestData {
            token_id,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_address = user_no_token_balance.address();
    const BURN_AMOUNT: u128 = 1;

    let initial_total_supply = runner.query_visible_state(|state| {
        Bank::<S>::default()
            .get_total_supply_of(&token_id, state)
            .unwrap_infallible()
            .unwrap()
    });

    runner.execute_transaction(TransactionTestCase {
        input: user_no_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: BURN_AMOUNT.into(),
                    token_id,
                },
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(
                matches!(result.tx_receipt, TxEffect::Reverted(..)),
                "The transaction should have been reverted"
            );

            // Burn by another user, who doesn't have tokens at all
            match result.tx_receipt {
                TxEffect::Reverted(contents) => {
                    let Error::ModuleError(err) = contents.reason;
                    let mut chain = err.chain();

                    let message_1 = chain.next().unwrap().to_string();
                    let message_2 = chain.next().unwrap().to_string();

                    assert!(chain.next().is_none());
                    assert_eq!(
                        message_1,
                        format!("Failed to burn token_id={token_id} owner={user_address}")
                    );

                    assert_eq!(
                        format!(
                            "Insufficient balance from={user_address}, got=0, needed={BURN_AMOUNT}",
                        ),
                        message_2,
                        "The error message is incorrect"
                    );
                }
                _ => {
                    panic!("The transaction should have been reverted")
                }
            }

            let final_total_supply = Bank::<S>::default()
                .get_total_supply_of(&token_id, state)
                .unwrap_infallible()
                .unwrap();

            assert_eq!(
                initial_total_supply, final_total_supply,
                "The token supply shouldn't have changed"
            );
        }),
    });
}

#[test]
fn burn_more_than_deployed_tokens_fails() {
    let (
        TestData {
            token_id,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let total_token_supply = runner.query_visible_state(|state| {
        Bank::<S>::default()
            .get_total_supply_of(&token_id, state)
            .unwrap_infallible()
            .unwrap()
    });

    let to_burn = total_token_supply.checked_add(Amount::new(1)).unwrap();

    let user_address = user_high_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: to_burn,
                    token_id,
                },
            },
        ),
        assert: Box::new(move |result, state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();

                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();

                assert!(chain.next().is_none());
                assert_eq!(
                    message_1,
                    format!("Failed to burn token_id={token_id} owner={user_address}",)
                );

                assert_eq!(
                    format!(
                        "Total supply underflow when burning, supply=300000 is less than burn amount={to_burn}",
                    ),
                    message_2,
                    "The error message is incorrect"
                );

                let final_total_supply = Bank::<S>::default()
                    .get_total_supply_of(&token_id, state)
                    .unwrap_infallible()
                    .unwrap();

                assert_eq!(
                    total_token_supply, final_total_supply,
                    "The token supply shouldn't have changed"
                );
            }
            _ => panic!("The outcome is incorrect"),
        }),
    });
}

#[test]
fn burn_more_than_available_balance_fails() {
    let (
        TestData {
            token_id,
            token_name,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let initial_token_supply = runner.query_visible_state(|state| {
        Bank::<S>::default()
            .get_total_supply_of(&token_id, state)
            .unwrap_infallible()
            .unwrap()
    });

    let user_token_balance = user_high_token_balance.token_balance(&token_name).unwrap();

    let user_address = user_high_token_balance.address();

    let to_burn = user_token_balance.checked_add(Amount::new(1)).unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: to_burn,
                    token_id,
                },
            },
        ),
        assert: Box::new(move |result, state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();

                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();

                assert!(chain.next().is_none());
                assert_eq!(
                    message_1,
                    format!(
                        "Failed to burn token_id={token_id} owner={user_address}"
                    )
                );

                assert_eq!(
                    format!(
                        "Insufficient balance from={user_address}, got={user_token_balance}, needed={to_burn}",
                    ),
                    message_2,
                    "The error message is incorrect"
                );

                let final_total_supply = Bank::<S>::default()
                    .get_total_supply_of(&token_id, state)
                    .unwrap_infallible()
                    .unwrap();

                assert_eq!(
                    initial_token_supply, final_total_supply,
                    "The token supply shouldn't have changed"
                );
            }
            _ => {
                panic!("The transaction does not have the expected outcome.")
            }
        }),
    });
}

#[test]
fn test_burning_zero_tokens_works() {
    let (
        TestData {
            token_name,
            token_id,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_token_balance = user_high_token_balance.token_balance(&token_name).unwrap();

    let user_address = user_high_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: Amount::ZERO,
                    token_id,
                },
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                TestBankRuntimeEvent::Bank(Event::TokenBurned {
                    owner: TokenHolder::User(user_address),
                    coins: Coins {
                        amount: Amount::ZERO,
                        token_id
                    }
                }),
                result.events[0]
            );

            // Check that the user's balance hasn't changed
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_address, token_id, state)
                    .unwrap_infallible(),
                Some(user_token_balance),
                "The user's balance shouldn't have changed"
            );
        }),
    });
}

#[test]
fn test_burning_zero_tokens_for_user_with_no_balance() {
    let (
        TestData {
            token_id,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_address = user_no_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_no_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: Amount::ZERO,
                    token_id,
                },
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                TestBankRuntimeEvent::Bank(Event::TokenBurned {
                    owner: TokenHolder::User(user_address),
                    coins: Coins {
                        amount: Amount::ZERO,
                        token_id
                    }
                }),
                result.events[0]
            );

            // Check that the user's balance hasn't changed
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_address, token_id, state)
                    .unwrap_infallible(),
                Some(Amount::ZERO),
                "The user's balance shouldn't have changed"
            );
        }),
    });
}

#[test]
fn burn_unknown_token_fails() {
    let (
        TestData {
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    const AMOUNT_TO_BURN: u128 = 0;

    let other_token_id = TestTokenName::new("OtherToken".to_string()).id();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    amount: AMOUNT_TO_BURN.into(),
                    token_id: other_token_id,
                },
            },
        ),
        assert: Box::new(move |result, _| {
            assert!(
                matches!(result.tx_receipt, TxEffect::Reverted(..)),
                "The transaction should have been reverted"
            );

            match result.tx_receipt {
                TxEffect::Reverted(contents) => {
                    let Error::ModuleError(err) = contents.reason;
                    let mut chain = err.chain();

                    let message_1 = chain.next().unwrap().to_string();
                    let message_2 = chain.next().unwrap().to_string();
                    let message_3 = chain.next().unwrap().to_string();

                    assert!(chain.next().is_none());

                    assert_eq!(
                        format!(
                            "Failed to burn token_id={other_token_id} owner={}",
                            user_high_token_balance.address()
                        ),
                        message_1,
                        "The first message is incorrect"
                    );

                    assert_eq!(
                        format!("Failed to get token_id={other_token_id}"),
                        message_2,
                        "The second message is incorrect"
                    );

                    // Note, no token ID in the root cause the message.
                    let expected_error_part =
                        "Value not found for prefix: \"sov_bank/Bank/tokens/\" and storage key:";
                    assert!(message_3.starts_with(expected_error_part));
                }
                _ => {
                    panic!("The transaction does not have the expected outcome.")
                }
            }
        }),
    });
}

/// Simple test to check that burning the gas token works.
#[test]
fn burn_gas_token_also_works() {
    let (
        TestData {
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_gas_balance = user_high_token_balance.available_gas_balance;
    let user_address = user_high_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Burn {
                coins: Coins {
                    // Note: we are only burning half of the gas balance because some of it is
                    // already consumed (for pre-execution checks) by the time we are reaching the burn method of the `Bank` module.
                    amount: in_half(user_gas_balance),
                    token_id: config_gas_token_id(),
                },
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                TestBankRuntimeEvent::Bank(Event::TokenBurned {
                    owner: TokenHolder::User(user_address),
                    coins: Coins {
                        amount: in_half(user_gas_balance),
                        token_id: config_gas_token_id()
                    }
                }),
                result.events[0]
            );

            // Check that the user's gas balance is now equal to the burnt amount minus the gas used to send the transaction
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_address, config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    in_half(user_gas_balance)
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                ),
                "The user's balance should be zero"
            );
        }),
    });
}

fn in_half(amount: Amount) -> Amount {
    amount.checked_div(Amount::new(2)).unwrap()
}
