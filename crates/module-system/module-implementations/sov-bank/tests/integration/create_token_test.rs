use sov_bank::utils::TokenHolder;
use sov_bank::{get_token_id, Amount, Bank};
use sov_modules_api::TxEffect;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::genesis::TestTokenName;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::helpers::*;

type S = sov_test_utils::TestSpec;
const INITIAL_TOKEN_BALANCE: Amount = Amount::new(1000);

// Check that we can create a token and that the state is correctly updated.
#[test]
fn create_token() {
    let (
        TestData {
            minter,
            user_high_token_balance,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_high_token_balance_address = user_high_token_balance.address();
    let user_no_token_balance_address = user_no_token_balance.address();
    let minter_address = minter.as_user().address();
    let token_name = "Token1";
    let token_id = get_token_id::<S>(token_name, None, &minter_address);

    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::CreateToken {
            token_name: token_name.try_into().unwrap(),
            token_decimals: None,
            initial_balance: INITIAL_TOKEN_BALANCE,
            mint_to_address: user_high_token_balance_address,
            supply_cap: Some(INITIAL_TOKEN_BALANCE),
            admins: vec![user_no_token_balance_address, minter_address]
                .try_into()
                .expect("Tokens can have at least one minter"),
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1, "There should be one event emitted");
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenCreated {
                    token_name: token_name.to_string(),
                    coins: sov_bank::Coins {
                        amount: INITIAL_TOKEN_BALANCE,
                        token_id
                    },
                    minter: TokenHolder::User(minter.address()),
                    mint_to_address: TokenHolder::User(user_high_token_balance_address),
                    admins: vec![
                        TokenHolder::User(user_no_token_balance_address),
                        TokenHolder::User(minter_address)
                    ],
                    supply_cap: INITIAL_TOKEN_BALANCE,
                }),
                "The event should be a TokenCreated event"
            );

            let token = Bank::<S>::default()
                .get_token(&token_id, state)
                .unwrap()
                .unwrap();

            assert_eq!(token.name(), token_name);
            assert_eq!(token.total_supply(), INITIAL_TOKEN_BALANCE);
            assert_eq!(
                token.admins(),
                [
                    TokenHolder::User(user_no_token_balance_address),
                    TokenHolder::User(minter_address)
                ]
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_high_token_balance_address, token_id, state)
                    .unwrap(),
                Some(INITIAL_TOKEN_BALANCE)
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&minter_address, token_id, state)
                    .unwrap(),
                None,
                "The minter should not receive any tokens! It should only be able to mint"
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_token_balance_address, token_id, state)
                    .unwrap(),
                None
            );
        }),
    });
}

/// Check that we can create a token and mint them to a user.
#[test]
fn create_token_and_mint() {
    let (
        TestData {
            minter,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_no_token_balance_address = user_no_token_balance.address();
    let minter_address = minter.as_user().address();
    let token_name = "Token1";
    let token_id = get_token_id::<S>(token_name, None, &minter_address);

    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::CreateToken {
            token_name: token_name.try_into().unwrap(),
            token_decimals: None,
            initial_balance: INITIAL_TOKEN_BALANCE,
            mint_to_address: minter_address,
            supply_cap: None,
            admins: vec![minter_address]
                .try_into()
                .expect("Tokens can have at least one minter"),
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1, "There should be one event emitted");
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenCreated {
                    token_name: token_name.to_string(),
                    coins: sov_bank::Coins {
                        amount: INITIAL_TOKEN_BALANCE,
                        token_id
                    },
                    minter: sov_bank::utils::TokenHolder::User(minter_address),
                    mint_to_address: sov_bank::utils::TokenHolder::User(minter_address),
                    admins: vec![sov_bank::utils::TokenHolder::User(minter_address)],
                    supply_cap: Amount::MAX,
                }),
                "The event should be a TokenCreated event"
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_token_balance_address, token_id, state)
                    .unwrap(),
                None
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Mint {
            coins: sov_bank::Coins {
                amount: INITIAL_TOKEN_BALANCE,
                token_id,
            },
            mint_to_address: user_no_token_balance_address,
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1, "There should be one event emitted");
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenMinted {
                    mint_to_identity: sov_bank::utils::TokenHolder::User(
                        user_no_token_balance_address
                    ),
                    authorizer: sov_bank::utils::TokenHolder::User(minter_address),
                    coins: sov_bank::Coins {
                        amount: INITIAL_TOKEN_BALANCE,
                        token_id
                    }
                }),
                "The event should be a TokenMinted event"
            );
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_token_balance_address, token_id, state)
                    .unwrap(),
                Some(INITIAL_TOKEN_BALANCE)
            );

            // The minter should have the same balance because the tokens were minted and not transferred
            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&minter_address, token_id, state)
                    .unwrap(),
                Some(INITIAL_TOKEN_BALANCE)
            );

            // The total supply should have increased by the amount of tokens minted
            assert_eq!(
                Bank::<S>::default()
                    .get_total_supply_of(&token_id, state)
                    .unwrap(),
                Some(INITIAL_TOKEN_BALANCE.checked_mul(Amount::new(2)).unwrap())
            );
        }),
    });
}

/// Check that we can create a token and mint them to a user.
#[test]
fn create_token_and_mint_fails_if_exceeds_supply_cap() {
    let (
        TestData {
            minter,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_no_token_balance_address = user_no_token_balance.address();
    let minter_address = minter.as_user().address();
    let token_name = "Token1";
    let token_id = get_token_id::<S>(token_name, None, &minter_address);

    // Try to create a token and mint more than the supply cap. SHuld fail
    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::CreateToken {
            token_name: token_name.try_into().unwrap(),
            token_decimals: None,
            initial_balance: INITIAL_TOKEN_BALANCE,
            mint_to_address: minter_address,
            supply_cap: Some(INITIAL_TOKEN_BALANCE.checked_sub(Amount::new(1)).unwrap()),
            admins: vec![minter_address]
                .try_into()
                .expect("Tokens can have at least one minter"),
        }),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });

    // Try to create a token with the correct supply cap. Should succeed
    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::CreateToken {
            token_name: token_name.try_into().unwrap(),
            token_decimals: None,
            initial_balance: INITIAL_TOKEN_BALANCE,
            mint_to_address: minter_address,
            supply_cap: Some(INITIAL_TOKEN_BALANCE),
            admins: vec![minter_address]
                .try_into()
                .expect("Tokens can have at least one minter"),
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            assert_eq!(result.events.len(), 1, "There should be one event emitted");
            assert_eq!(
                result.events[0],
                TestBankRuntimeEvent::Bank(sov_bank::event::Event::TokenCreated {
                    token_name: token_name.to_string(),
                    coins: sov_bank::Coins {
                        amount: INITIAL_TOKEN_BALANCE,
                        token_id
                    },
                    minter: sov_bank::utils::TokenHolder::User(minter_address),
                    mint_to_address: sov_bank::utils::TokenHolder::User(minter_address),
                    admins: vec![sov_bank::utils::TokenHolder::User(minter_address)],
                    supply_cap: INITIAL_TOKEN_BALANCE,
                }),
                "The event should be a TokenCreated event"
            );

            assert_eq!(
                Bank::<S>::default()
                    .get_balance_of(&user_no_token_balance_address, token_id, state)
                    .unwrap(),
                None
            );
        }),
    });

    // Try to mint more than the supply cap. Should fail
    runner.execute_transaction(TransactionTestCase {
        input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::Mint {
            coins: sov_bank::Coins {
                amount: Amount::new(1),
                token_id,
            },
            mint_to_address: user_no_token_balance_address,
        }),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });
}

#[test]
fn test_create_token_fails_with_duplicate_ids() {
    let (
        TestData {
            minter,
            user_high_token_balance,
            user_no_token_balance,
            ..
        },
        mut runner,
    ) = setup();

    let user_high_token_balance_address = user_high_token_balance.address();
    let user_no_token_balance_address = user_no_token_balance.address();
    let minter_address = minter.as_user().address();
    let token_name = "Token1";
    let token_id = get_token_id::<S>(token_name, None, &minter_address);

    runner
        .execute_transaction(TransactionTestCase {
            input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::CreateToken {
                token_name: token_name.try_into().unwrap(),
                token_decimals: None,
                initial_balance: INITIAL_TOKEN_BALANCE,
                mint_to_address: user_high_token_balance_address,
                supply_cap: None,
                admins: vec![user_no_token_balance_address, minter_address]
                    .try_into()
                    .expect("Tokens can have at least one minter"),
            }),
            assert: Box::new(move |result, _state| {
                assert!(result.tx_receipt.is_successful());
            }),
        })
        .execute_transaction(TransactionTestCase {
            input: minter.create_plain_message::<RT, Bank<S>>(sov_bank::CallMessage::CreateToken {
                token_name: token_name.try_into().unwrap(),
                token_decimals: None,
                initial_balance: INITIAL_TOKEN_BALANCE,
                mint_to_address: user_high_token_balance_address,
                supply_cap: None,
                admins: vec![user_no_token_balance_address, minter_address]
                    .try_into()
                    .expect("Tokens can have at least one minter"),
            }),
            assert: Box::new(move |result, _state| {
                if let TxEffect::Reverted(contents) = result.tx_receipt {
                    let sov_modules_api::Error::ModuleError(err) = contents.reason;
                    assert_eq!(
                        err.to_string(),
                        format!(
                            "Token with id already exists {}, name={} minter={}",
                            token_id,
                            token_name,
                            minter.address()
                        )
                    );
                } else {
                    panic!("The transaction should have failed");
                }
            }),
        });
}

#[test]
#[should_panic]
fn overflow_max_supply_genesis_should_panic() {
    let token_name = TestTokenName::new("BankToken".to_string());
    let genesis_config = HighLevelOptimisticGenesisConfig::generate()
        .add_accounts_with_default_balance(1)
        .add_accounts_with_token(
            &token_name,
            false,
            2,
            Amount::MAX.checked_sub(Amount::new(2)).unwrap(),
        );

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());

    TestRunner::<_, _>::new_with_genesis(genesis.into_genesis_params(), RT::default());
}
