use sov_bank::{Bank, CallMessage, Coins, TokenId};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, Error, SafeVec, TxEffect};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::helpers::{setup, TestBankRuntimeEvent, TestData, RT};

type S = sov_test_utils::TestSpec;

const MINT_AMOUNT: Amount = Amount::new(100);
const INITIAL_BALANCE: Amount = Amount::new(100);

/// Tests that a user can mint tokens.
#[test]
fn mint_token_success() {
    let (
        TestData {
            token_id,
            minter,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(CallMessage::Mint {
            coins: Coins {
                amount: MINT_AMOUNT,
                token_id,
            },
            mint_to_address: user_no_token_balance.address(),
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1);
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenMinted {
                    mint_to_identity: sov_bank::utils::TokenHolder::User(
                        user_no_token_balance.address()
                    ),
                    authorizer: sov_bank::utils::TokenHolder::User(minter.address()),
                    coins: Coins {
                        amount: MINT_AMOUNT,
                        token_id
                    }
                })
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_token_balance.address(), token_id, state)
                    .unwrap_infallible(),
                Some(MINT_AMOUNT)
            );
        }),
    });
}

#[test]
fn mint_token_fails_if_user_unauthorized() {
    let (
        TestData {
            token_name,
            token_id,
            user_high_token_balance: unauthorized_minter,
            ..
        },
        mut runner,
    ) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: unauthorized_minter.create_plain_message::<RT, Bank<S>>(CallMessage::Mint {
            coins: Coins {
                amount: Amount::ZERO,
                token_id,
            },
            mint_to_address: unauthorized_minter.address(),
        }),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(message_1, format!("Failed to mint token_id={token_id}"));
                assert_eq!(
                    message_2,
                    format!(
                        "Sender {} is not an admin of token {}",
                        unauthorized_minter.address(),
                        token_name
                    ),
                );
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

/// The token originator shouldn't be able to mint tokens he created.
#[test]
fn try_create_token_and_mint_should_fail_if_not_authorized() {
    let (
        TestData {
            token_name,
            token_id,
            user_no_token_balance: user,
            ..
        },
        mut runner,
    ) = setup();

    runner.execute(
        user.create_plain_message::<RT, Bank<S>>(CallMessage::CreateToken {
            token_name: token_name.to_string().try_into().unwrap(),
            token_decimals: None,
            supply_cap: None,
            initial_balance: Amount::new(100),
            mint_to_address: user.address(),
            admins: SafeVec::new(),
        }),
    );

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Bank<S>>(CallMessage::Mint {
            coins: Coins {
                amount: INITIAL_BALANCE,
                token_id,
            },
            mint_to_address: user.address(),
        }),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(message_1, format!("Failed to mint token_id={token_id}"));
                assert_eq!(
                    message_2,
                    format!(
                        "Sender {} is not an admin of token {}",
                        user.address(),
                        token_name
                    ),
                );
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

#[test]
fn mint_token_account_balance_overflow() {
    let (
        TestData {
            token_id, minter, ..
        },
        mut runner,
    ) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(CallMessage::Mint {
            coins: Coins {
                amount: Amount::MAX,
                token_id,
            },
            mint_to_address: minter.address(),
        }),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(message_1, format!("Failed to mint token_id={token_id}"));
                assert_eq!(
                    message_2,
                    "Total Supply overflow in the mint method of bank module",
                );
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

#[test]
fn mint_token_total_supply_overflow() {
    let (
        TestData {
            token_name,
            token_id,
            minter,
            ..
        },
        mut runner,
    ) = setup();

    let minter_balance = minter.token_balance(&token_name).unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(CallMessage::Mint {
            coins: Coins {
                amount: Amount::MAX
                    .checked_sub(minter_balance)
                    .unwrap()
                    .checked_sub(Amount::new(1))
                    .unwrap(),
                token_id,
            },
            mint_to_address: minter.address(),
        }),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                let message_2 = chain.next().unwrap().to_string();
                assert!(chain.next().is_none());
                assert_eq!(message_1, format!("Failed to mint token_id={token_id}"));
                assert_eq!(
                    message_2,
                    "Total Supply overflow in the mint method of bank module",
                );
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

#[test]
fn test_mint_token_fails_if_token_doesnt_exist() {
    let (
        TestData {
            user_high_token_balance: unauthorized_minter,
            ..
        },
        mut runner,
    ) = setup();
    let invalid_token_id = TokenId::generate::<S>("invalid");

    runner.execute_transaction(TransactionTestCase {
        input: unauthorized_minter.create_plain_message::<RT, Bank<S>>(CallMessage::Mint {
            coins: Coins {
                amount: Amount::new(50),
                token_id: invalid_token_id,
            },
            mint_to_address: unauthorized_minter.address(),
        }),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                let mut chain = err.chain();
                let message_1 = chain.next().unwrap().to_string();
                assert_eq!(
                    message_1,
                    format!("Failed to get token_id={invalid_token_id}")
                );
            } else {
                panic!("The transaction should have failed");
            }
        }),
    });
}

#[test]
fn test_mint_token_fails_if_token_is_frozen() {
    let (
        TestData {
            token_id,
            token_name,
            minter,
            ..
        },
        mut runner,
    ) = setup();

    runner
        .execute_transaction(TransactionTestCase {
            input: minter.create_plain_message::<RT, Bank<S>>(CallMessage::Freeze { token_id }),
            assert: Box::new(move |result, _state| {
                assert!(result.tx_receipt.is_successful());
            }),
        })
        .execute_transaction(TransactionTestCase {
            input: minter.create_plain_message::<RT, Bank<S>>(CallMessage::Mint {
                coins: Coins {
                    amount: Amount::new(50),
                    token_id,
                },
                mint_to_address: minter.address(),
            }),
            assert: Box::new(move |result, _state| {
                if let TxEffect::Reverted(contents) = result.tx_receipt {
                    let Error::ModuleError(err) = contents.reason;
                    let mut chain = err.chain();
                    let message_1 = chain.next().unwrap().to_string();
                    let message_2 = chain.next().unwrap().to_string();
                    assert_eq!(message_1, format!("Failed to mint token_id={token_id}"));
                    assert_eq!(
                        message_2,
                        format!("Attempt to mint frozen token {token_name}")
                    );
                } else {
                    panic!("The transaction should have failed");
                }
            }),
        });
}
