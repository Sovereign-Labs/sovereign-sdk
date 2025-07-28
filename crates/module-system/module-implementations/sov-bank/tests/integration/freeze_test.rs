use sov_bank::{Bank, TokenId};
use sov_modules_api::{Amount, Error, TxEffect};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::helpers::{setup, TestBankRuntimeEvent, TestData, RT, S};

/// Check that the admin can freeze a token
#[test]
fn freeze_token_happy_path() {
    let (
        TestData {
            minter,
            token_id,
            token_name,
            ..
        },
        mut runner,
    ) = setup();

    let minter_address = minter.as_user().address();

    runner.execute_transaction(TransactionTestCase {
        input: minter
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Freeze { token_id }),
        assert: Box::new(move |result, _| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenFrozen {
                    freezer: sov_bank::utils::TokenHolder::User(minter_address),
                    token_id
                }),
                "The event should be a TokenFrozen event"
            );
        }),
    });

    // We can check that the token is frozen by trying to mint
    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Mint {
            coins: sov_bank::Coins {
                amount: Amount::ZERO,
                token_id,
            },
            mint_to_address: minter_address,
        }),
        assert: Box::new(move |result, _| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(message_1, format!("Failed to mint token_id={token_id}"));
                assert_eq!(
                    format!("Attempt to mint frozen token {token_name}"),
                    message_2
                );
            } else {
                panic!("The transaction should have reverted");
            }
        }),
    });
}

#[test]
fn freeze_another_time_fails() {
    let (
        TestData {
            minter,
            token_id,
            token_name,
            ..
        },
        mut runner,
    ) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: minter
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Freeze { token_id }),
        assert: Box::new(move |result, _| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // We cannot freeze the token again
    runner.execute_transaction(TransactionTestCase {
        input: minter
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Freeze { token_id }),
        assert: Box::new(move |result, _| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(format!("Failed to freeze token_id={token_id}"), message_1);
                assert_eq!(format!("Token {token_name} is already frozen"), message_2);
            } else {
                panic!("The transaction should have reverted");
            }
        }),
    });
}

#[test]
fn unauthorized_minter_cannot_freeze_token() {
    let (
        TestData {
            user_high_token_balance: unauthorized_user,
            token_id,
            token_name,
            ..
        },
        mut runner,
    ) = setup();

    assert!(!unauthorized_user.is_minter(&token_name));

    let unauthorized_address = unauthorized_user.as_user().address();

    runner.execute_transaction(TransactionTestCase {
        input: unauthorized_user
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Freeze { token_id }),
        assert: Box::new(move |result, _| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(format!("Failed to freeze token_id={token_id}"), message_1);
                assert_eq!(
                    format!("Sender {unauthorized_address} is not an admin of token {token_name}"),
                    message_2
                );
            } else {
                panic!("The transaction should have reverted");
            }
        }),
    });
}

#[test]
fn test_freeze_fails_if_token_id_doesnt_exist() {
    let (TestData { minter, .. }, mut runner) = setup();

    let token_id = TokenId::generate::<S>("invalid");

    runner.execute_transaction(TransactionTestCase {
        input: minter
            .create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Freeze { token_id }),
        assert: Box::new(move |result, _| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                assert_eq!(format!("Failed to get token_id={token_id}"), message_1);
            } else {
                panic!("The transaction should have reverted");
            }
        }),
    });
}
