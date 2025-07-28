use sov_bank::utils::TokenHolder;
use sov_bank::{config_gas_token_id, Bank, CallMessage, Coins};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, Error, TxEffect};
use sov_test_utils::runtime::genesis::TestTokenName;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase};

use crate::helpers::*;

type S = sov_test_utils::TestSpec;

const TRANSFER_AMOUNT: Amount = Amount::new(10);

/// Tests the happy path of a transfer call. Transfer a given amount of tokens from a user with a high balance to another user.
#[test]
fn transfer_token_happy_path() {
    let (
        TestData {
            token_id,
            token_name,
            user_high_token_balance,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_high_token_balance_address = user_high_token_balance.address();
    let user_high_token_initial_balance =
        user_high_token_balance.token_balance(&token_name).unwrap();

    let user_no_token_balance_address = user_no_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: user_no_token_balance_address,
            coins: Coins {
                amount: TRANSFER_AMOUNT,
                token_id,
            },
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenTransferred {
                    from: TokenHolder::User(user_high_token_balance_address),
                    to: TokenHolder::User(user_no_token_balance_address),
                    coins: Coins {
                        amount: TRANSFER_AMOUNT,
                        token_id
                    }
                })
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_token_balance_address, token_id, state)
                    .unwrap_infallible(),
                Some(TRANSFER_AMOUNT)
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_high_token_balance_address, token_id, state)
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

/// Tests that a transfer call fails when the sender does not have enough balance.
#[test]
fn transfer_balance_too_low() {
    let (
        TestData {
            token_id,
            token_name,
            user_high_token_balance,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_high_token_balance_address = user_high_token_balance.address();
    let user_high_token_initial_balance =
        user_high_token_balance.token_balance(&token_name).unwrap();

    let user_no_token_balance_address = user_no_token_balance.address();

    let transfer_amount = user_high_token_initial_balance
        .checked_add(Amount::new(1))
        .unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: user_no_token_balance_address,
            coins: Coins {
                amount: transfer_amount,
                token_id,
            },
        }),
        assert: Box::new(move |result, state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(
                    format!("Failed to transfer token_id={token_id}",),
                    message_1
                );
                assert_eq!(
                    format!(
                        "Insufficient balance from={user_high_token_balance_address}, got={user_high_token_initial_balance}, needed={transfer_amount}",
                    ),
                    message_2,
                );

                assert_eq!(
                    Bank::<S>::default()
                        .get_balance_of(&user_high_token_balance_address, token_id, state)
                        .unwrap_infallible(),
                    Some(user_high_token_initial_balance)
                );

                assert_eq!(
                    Bank::<S>::default()
                        .get_balance_of(&user_no_token_balance_address, token_id, state)
                        .unwrap_infallible(),
                    None
                );
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

/// Test that a transfer call fails when the token does not exist.
#[test]
fn transfer_non_existent_token() {
    let (
        TestData {
            user_high_token_balance: user,
            ..
        },
        mut runner,
    ) = setup();

    let non_existent_token = TestTokenName::new("NonExistentToken".to_string());

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: sov_modules_api::Address::new([1u8; 28]),
            coins: Coins {
                amount: Amount::new(1),
                token_id: non_existent_token.id(),
            },
        }),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                println!("{message_1}\n{message_2}");
                assert!(chain.next().is_none());

                assert!(message_1.starts_with(
                    "Failed to transfer token_id=token_1ry733wdf5jt2hkgyljcgy54k3julqqtvrf9j2wfty0l7tnrrdqyqq4a0a3"
                ));
                assert!(message_2.starts_with(
                    "Insufficient balance from="
                ));
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

/// Test that a transfer call fails when the sender does not have any balance for the token.
#[test]
fn transfer_sender_does_not_have_balance() {
    let (
        TestData {
            token_id,
            user_no_token_balance,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let sender_address = user_no_token_balance.address();
    let receiver_address = user_high_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_no_token_balance.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: receiver_address,
            coins: Coins {
                amount: TRANSFER_AMOUNT,
                token_id,
            },
        }),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());

                assert_eq!(
                    format!("Failed to transfer token_id={token_id}"),
                    message_1
                );

                assert_eq!(
                    format!(
                        "Insufficient balance from={sender_address}, got=0, needed={TRANSFER_AMOUNT}",
                    ),
                    message_2,
                    "The error message is incorrect"
                );
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

/// Test that a transfer call succeeds even when the receiver does not exist.
#[test]
fn transfer_receiver_does_not_have_balance() {
    let (
        TestData {
            token_id,
            token_name,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let sender_address = user_high_token_balance.address();
    let sender_initial_balance = user_high_token_balance.token_balance(&token_name).unwrap();

    // Generate a new user with a zero balance
    let receiver_address = TestUser::<S>::generate(Amount::ZERO).address();

    runner.execute_transaction(TransactionTestCase {
        input: user_high_token_balance.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: receiver_address,
            coins: Coins {
                amount: TRANSFER_AMOUNT,
                token_id,
            },
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenTransferred {
                    from: TokenHolder::User(sender_address),
                    to: TokenHolder::User(receiver_address),
                    coins: Coins {
                        amount: TRANSFER_AMOUNT,
                        token_id
                    }
                })
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&receiver_address, token_id, state)
                    .unwrap_infallible(),
                Some(TRANSFER_AMOUNT)
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&sender_address, token_id, state)
                    .unwrap_infallible(),
                Some(sender_initial_balance.checked_sub(TRANSFER_AMOUNT).unwrap())
            );
        }),
    });
}

#[test]
fn transfer_sender_equals_receiver_zero_balance() {
    let (
        TestData {
            token_id,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let sender_address = user_no_token_balance.address();

    // Transfer fails if the sender sends to itself and TRANSFER_AMOUNT > 0
    runner.execute_transaction(TransactionTestCase {
        input: user_no_token_balance.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: sender_address,
            coins: Coins {
                amount: TRANSFER_AMOUNT,
                token_id,
            },
        }),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_reverted());
            assert!(result.events.is_empty());
        }),
    });

    // Transfer succeeds if the sender sends to itself and TRANSFER_AMOUNT = 0
    let zero = Amount::ZERO;
    runner.execute_transaction(TransactionTestCase {
        input: user_no_token_balance.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: sender_address,
            coins: Coins {
                amount: zero,
                token_id,
            },
        }),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenTransferred {
                    from: TokenHolder::User(sender_address),
                    to: TokenHolder::User(sender_address),
                    coins: Coins {
                        amount: zero,
                        token_id
                    }
                })
            );
        }),
    });
}

/// Sender sends tokens to itself.
#[test]
fn transfer_sender_equals_receiver() {
    let (
        TestData {
            token_id,
            token_name,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let sender_address = user_high_token_balance.address();
    let sender_initial_balance = user_high_token_balance.token_balance(&token_name).unwrap();

    // Transfer to self fails if the amount exceeds the sender's balance.
    {
        let amount = sender_initial_balance.saturating_add(Amount::new(1));

        runner.execute_transaction(TransactionTestCase {
            input: user_high_token_balance.create_plain_message::<RT, Bank<S>>(
                CallMessage::Transfer {
                    to: sender_address,
                    coins: Coins { amount, token_id },
                },
            ),
            assert: Box::new(move |result, _state| {
                assert!(result.tx_receipt.is_reverted());
                assert_eq!(result.events.len(), 0);
            }),
        });
    }

    // Transfer to self succeeds if the amount equals the sender's balance.
    {
        let amount = sender_initial_balance;
        runner.execute_transaction(TransactionTestCase {
            input: user_high_token_balance.create_plain_message::<RT, Bank<S>>(
                CallMessage::Transfer {
                    to: sender_address,
                    coins: Coins { amount, token_id },
                },
            ),
            assert: Box::new(move |result, _state| {
                assert!(result.tx_receipt.is_successful());
                assert_eq!(result.events.len(), 1);
                assert_eq!(
                    result.events[0],
                    TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenTransferred {
                        from: TokenHolder::User(sender_address),
                        to: TokenHolder::User(sender_address),
                        coins: Coins { amount, token_id }
                    })
                );
            }),
        });
    }
}

/// Test that a transfer call succeeds when the sender sends a null amount of a valid token. Even if the sender has zero balance.
#[test]
fn transfer_send_zero_amount() {
    let (
        TestData {
            token_id,
            user_no_token_balance,
            user_high_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let sender_address = user_no_token_balance.address();
    let receiver_address = user_high_token_balance.address();

    runner.execute_transaction(TransactionTestCase {
        input: user_no_token_balance.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: receiver_address,
            coins: Coins {
                amount: Amount::ZERO,
                token_id,
            },
        }),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenTransferred {
                    from: TokenHolder::User(sender_address),
                    to: TokenHolder::User(receiver_address),
                    coins: Coins {
                        amount: Amount::ZERO,
                        token_id
                    }
                })
            );
        }),
    });
}

/// Test that it is possible to transfer gas tokens
#[test]
fn test_transfer_gas_token() {
    let (
        TestData {
            user_high_token_balance: sender,
            user_no_token_balance: receiver,
            ..
        },
        mut runner,
    ) = setup();

    let sender_address = sender.address();
    let sender_initial_balance = sender.available_gas_balance;

    let receiver_address = receiver.address();
    let receiver_initial_balance = receiver.available_gas_balance;

    runner.execute_transaction(TransactionTestCase {
        input: sender.create_plain_message::<RT, Bank<S>>(CallMessage::Transfer {
            to: receiver_address,
            coins: Coins {
                amount: TRANSFER_AMOUNT,
                token_id: config_gas_token_id(),
            },
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenTransferred {
                    from: TokenHolder::User(sender_address),
                    to: TokenHolder::User(receiver_address),
                    coins: Coins {
                        amount: TRANSFER_AMOUNT,
                        token_id: config_gas_token_id()
                    }
                })
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&receiver_address, config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    receiver_initial_balance
                        .checked_add(TRANSFER_AMOUNT)
                        .unwrap()
                )
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&sender_address, config_gas_token_id(), state)
                    .unwrap_infallible(),
                Some(
                    sender_initial_balance
                        .checked_sub(result.gas_value_used)
                        .unwrap()
                        .checked_sub(TRANSFER_AMOUNT)
                        .unwrap()
                )
            );
        }),
    });
}
