use sov_accounts::{Accounts, CallMessage, Response};
use sov_modules_api::{Error, PrivateKey, PublicKey, Spec, TxEffect};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_optimistic_runtime, AsUser, TestPrivateKey, TestUser, TransactionTestCase,
};

type S = sov_test_utils::TestSpec;

generate_optimistic_runtime!(TestAccountsRuntime <=);

type RT = TestAccountsRuntime<S>;

struct TestData<S: Spec> {
    account_1: TestUser<S>,
    account_2: TestUser<S>,
    non_registered_account: TestUser<S>,
}

/// We setup genesis with three accounts, two of which are registered at genesis.
fn setup() -> (TestData<S>, TestRunner<RT, S>) {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![
        TestUser::generate_with_default_balance().add_credential_id([0u8; 32].into()),
        TestUser::generate_with_default_balance().add_credential_id([1u8; 32].into()),
        TestUser::generate_with_default_balance(),
    ]);

    let user_1 = genesis_config.additional_accounts()[0].clone();
    let user_2 = genesis_config.additional_accounts()[1].clone();
    let user_3 = genesis_config.additional_accounts()[2].clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());

    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    (
        TestData {
            account_1: user_1,
            account_2: user_2,
            non_registered_account: user_3,
        },
        runner,
    )
}

#[test]
fn test_config_account() {
    let (
        TestData {
            account_1: user, ..
        },
        runner,
    ) = setup();

    // The account is registered at genesis.
    runner.query_visible_state(|state| {
        let accounts = Accounts::<S>::default();
        let response = accounts.get_account(user.credential_id(), state);
        assert_eq!(
            response,
            Response::AccountExists {
                addr: user.address()
            }
        );
    });
}

#[test]
fn test_update_account() {
    let (
        TestData {
            non_registered_account: user,
            ..
        },
        mut runner,
    ) = setup();

    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(
            new_credential,
        )),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            let accounts = Accounts::<S>::default();

            // New account with the new public key and an old address is created.
            assert_eq!(
                accounts.get_account(new_credential, state),
                Response::AccountExists {
                    addr: user.address()
                }
            );
            // Account corresponding to the old credential still exists.
            assert_eq!(
                accounts.get_account(user.credential_id(), state),
                Response::AccountExists {
                    addr: user.address()
                }
            );

            assert_ne!(new_credential, user.credential_id());
        }),
    });
}

#[test]
fn test_update_account_fails() {
    let (
        TestData {
            account_1,
            account_2,
            ..
        },
        mut runner,
    ) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: account_1.create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(
            account_2.credential_id(),
        )),
        assert: Box::new(move |result, _state| {
            if let TxEffect::Reverted(contents) = result.tx_receipt {
                let Error::ModuleError(err) = contents.reason;
                assert_eq!(err.to_string(), "New CredentialId already exists");
            }
        }),
    });
}

#[test]
fn test_register_new_account() {
    let (
        TestData {
            non_registered_account,
            ..
        },
        mut runner,
    ) = setup();

    // The account is empty at the start because it is not registered at genesis.
    assert_eq!(non_registered_account.custom_credential_id, None);

    runner.query_visible_state(|state| {
        let accounts = Accounts::<S>::default();
        let response = accounts.get_account(non_registered_account.credential_id(), state);
        assert_eq!(response, Response::AccountEmpty);
    });

    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: non_registered_account.create_plain_message::<RT, Accounts<S>>(
            CallMessage::InsertCredentialId(new_credential),
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            let accounts = Accounts::<S>::default();

            // New account with the new public key and an old address is created.
            assert_eq!(
                accounts.get_account(new_credential, state),
                Response::AccountExists {
                    addr: non_registered_account.address()
                }
            );

            // The default credential of the account exists
            assert_eq!(
                accounts.get_account(non_registered_account.credential_id(), state),
                Response::AccountExists {
                    addr: non_registered_account.address()
                }
            );

            assert_ne!(new_credential, non_registered_account.credential_id());
        }),
    });
}

#[test]
fn test_resolve_sender_address_with_default_address_non_registered() {
    let (
        TestData {
            non_registered_account,
            ..
        },
        runner,
    ) = setup();

    runner.query_visible_state(|state| {
        let mut accounts = Accounts::<S>::default();
        assert_eq!(
            accounts
                .resolve_sender_address(
                    &non_registered_account.address(),
                    &non_registered_account.credential_id(),
                    state
                )
                .unwrap(),
            non_registered_account.address()
        );
    });
}

#[test]
fn test_resolve_sender_address_registered() {
    let (
        TestData {
            account_1,
            account_2,
            ..
        },
        runner,
    ) = setup();

    runner.query_visible_state(|state| {
        let mut accounts = Accounts::<S>::default();

        // Ensure correct (registered) address is used even if another fallback is provided
        assert_eq!(
            accounts
                .resolve_sender_address(&account_2.address(), &account_1.credential_id(), state)
                .unwrap(),
            account_1.address()
        );
    });
}

/// Tests what happens if one tries to resolve an address when there is more than one credential available.
#[test]
fn test_resolve_address_if_more_than_one_credential() {
    let (
        TestData {
            non_registered_account,
            ..
        },
        mut runner,
    ) = setup();

    let pub_key_1 = TestPrivateKey::generate().pub_key();
    let credential_1 = pub_key_1.credential_id();
    let default_address_1 = credential_1.into();

    let pub_key_2 = TestPrivateKey::generate().pub_key();
    let credential_2 = pub_key_2.credential_id();
    let default_address_2 = credential_2.into();

    runner.execute(
        non_registered_account
            .create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(credential_1)),
    );

    runner.execute(
        non_registered_account
            .create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(credential_2)),
    );

    runner.query_visible_state(|state| {
        let mut accounts = Accounts::<S>::default();

        assert_eq!(
            accounts
                .resolve_sender_address(&default_address_1, &credential_1, state)
                .unwrap(),
            non_registered_account.address()
        );

        assert_eq!(
            accounts
                .resolve_sender_address(&default_address_2, &credential_2, state)
                .unwrap(),
            non_registered_account.address()
        );
    });
}

/// This test should verify that when a new credential is specified with an existing account's
/// address as fallback, that the credential is appended to that address. However
/// query_visible_state doesn't mutate the state so it simply verifies that the fallback address is
/// returned correctly
#[test]
fn test_resolve_with_different_default_address() {
    let (TestData { account_1, .. }, runner) = setup();

    let random_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.query_visible_state(|state| {
        let mut accounts = Accounts::<S>::default();

        assert_eq!(
            accounts
                .resolve_sender_address(&account_1.address(), &random_credential, state)
                .unwrap(),
            account_1.address()
        );
    });
}
