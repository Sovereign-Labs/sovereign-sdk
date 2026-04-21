use sov_accounts::{Accounts, CallMessage};
use sov_modules_api::transaction::{UnsignedTransactionV0, Version1};
use sov_modules_api::{
    CryptoSpec, PrivateKey, PublicKey, RawTx, Runtime, SkippedTxContents, Spec, TxEffect,
};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{default_test_tx_details, TransactionType};
use sov_test_utils::{
    generate_optimistic_runtime, AsUser, TestPrivateKey, TestUser, TransactionTestCase,
};

use sov_modules_api::transaction::PubKeyAndSignature;
use sov_modules_api::transaction::Transaction;

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

fn setup_with_disable_custom_account_mappings() -> (TestUser<S>, TestRunner<RT, S>) {
    let mut genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let user = TestUser::generate_with_default_balance();
    genesis_config = genesis_config.add_accounts(vec![user.clone()]);

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    genesis.accounts.enable_custom_account_mappings = false;
    (
        user,
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default()),
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

    // The account is registered at genesis: `(user.address(), user.credential_id())`
    // must appear in `account_owners`.
    runner.query_visible_state(|state| {
        let accounts = Accounts::<S>::default();
        assert!(
            accounts
                .is_authorized(&user.address(), &user.credential_id(), state)
                .unwrap(),
            "genesis-registered credential should be authorized for its address"
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

            // The new credential is authorized to spend as the sender's address.
            assert!(
                accounts
                    .is_authorized(&user.address(), &new_credential, state)
                    .unwrap(),
                "InsertCredentialId must authorize the new credential for the sender"
            );
            // The sender's own credential is still authorized (auto-registered on
            // first use when the tx was processed).
            assert!(
                accounts
                    .is_authorized(&user.address(), &user.credential_id(), state)
                    .unwrap(),
                "the sender's own credential stays authorized"
            );

            assert_ne!(new_credential, user.credential_id());
        }),
    });
}

/// In the new many-to-many model, authorizing another user's credential for
/// one's own address is a no-op (the attacker still needs the other party's
/// private key to sign). The duplicate guard only checks the exact
/// `(sender, credential)` tuple, so inserting a peer's credential succeeds.
/// A second `InsertCredentialId` with the same tuple must still revert.
#[test]
fn test_insert_peer_credential_succeeds_but_duplicate_fails() {
    let (
        TestData {
            account_1,
            non_registered_account: sender,
            ..
        },
        mut runner,
    ) = setup();

    let peer_credential = account_1.credential_id();
    let sender_address = sender.address();

    // First insert: authorizing a peer's credential for the sender's own
    // address is permitted.
    runner.execute_transaction(TransactionTestCase {
        input: sender.create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(
            peer_credential,
        )),
        assert: Box::new(move |result, state| {
            assert!(
                result.tx_receipt.is_successful(),
                "authorizing a peer's credential for your own address should succeed"
            );
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_authorized(&sender_address, &peer_credential, state)
                    .unwrap(),
                "peer credential should now be authorized for the sender's address"
            );
        }),
    });

    // Second insert with the same tuple reverts via the duplicate guard.
    runner.execute_transaction(TransactionTestCase {
        input: sender.create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(
            peer_credential,
        )),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "CredentialId already authorized for this address"
                );
            }
            _ => panic!("Expected reverted transaction on duplicate authorization"),
        }),
    });
}

/// Tests the multisig functionality of the Accounts module.
///
/// The multisig's effective signing address is its stateless default
/// (`multisig_credential_id.into()`). We seed genesis with a `TestUser` whose
/// custom `credential_id` matches the multisig, so the default address has a
/// gas balance and is already authorized in `account_owners` before any tx is
/// submitted. This keeps the focus on signature-level invariants.
#[test]
fn test_setup_multisig_and_act() {
    use sov_modules_api::Multisig;

    // Generate the multisig first so we know its credential_id at genesis time.
    let multisig_keys = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let multisig = Multisig::new(2, multisig_keys.iter().map(|k| k.pub_key()).collect());
    let multisig_credential_id =
        multisig.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();

    // Build a funded `TestUser` whose address is the multisig's default.
    let multisig_user =
        TestUser::generate_with_default_balance().add_credential_id(multisig_credential_id);

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user.clone()]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    // Define utilities for...
    // - Generating a valid multisig (version 1) transaction
    // - Signing a (version 1) transaction with a key
    // - Submitting the transaction and asserting it is successful
    // - Submitting the transaction and asserting it is skipped
    let generate_multisig_tx = || {
        let key = TestPrivateKey::generate();
        UnsignedTransactionV0::<RT, S>::new_with_details(
            TestAccountsRuntimeCall::Accounts(CallMessage::InsertCredentialId(
                key.pub_key().credential_id(),
            )),
            sov_modules_api::capabilities::UniquenessData::Generation(0),
            default_test_tx_details::<S>(),
        )
        .to_multisig_tx(multisig.clone())
    };

    let sign = |tx: &mut Version1<RT, S>, key: &TestPrivateKey| {
        use sov_modules_api::Runtime;
        let chain_hash = &<RT as Runtime<S>>::CHAIN_HASH;
        tx.sign(key, chain_hash).unwrap();
    };

    let assert_tx_success = |tx: Version1<RT, S>, runner: &mut TestRunner<RT, S>| {
        let tx = Transaction::<RT, S>::from(tx);
        let multisig_tx = TransactionType::<RT, S>::PreSigned(RawTx {
            data: borsh::to_vec(&tx).unwrap(),
        });
        runner.execute_transaction(TransactionTestCase {
            input: multisig_tx,
            assert: Box::new(move |result, _state| {
                assert!(result.tx_receipt.is_successful());
            }),
        });
    };

    let assert_tx_skip =
        |tx: Version1<RT, S>, runner: &mut TestRunner<RT, S>, reason: &'static str| {
            let tx = Transaction::<RT, S>::from(tx);
            let multisig_tx = TransactionType::<RT, S>::PreSigned(RawTx {
                data: borsh::to_vec(&tx).unwrap(),
            });
            runner.execute_transaction(TransactionTestCase {
                input: multisig_tx,
                assert: Box::new(move |result, _state| {
                    assert!(result.tx_receipt.is_skipped());
                    match result.tx_receipt {
                        TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                            assert!(
                                error.to_string().contains(reason),
                                "Unexpected skip reason. Expected: {reason}, Got: {error}"
                            );
                        }
                        _ => panic!(
                            "Expected skipped transaction but found {:?}",
                            result.tx_receipt
                        ),
                    }
                }),
            });
        };

    // A transaction with two valid signatures should succeed in our 2/3 multisig
    let tx_with_two_valid_signatures = {
        let mut tx = generate_multisig_tx();
        sign(&mut tx, &multisig_keys[0]);
        sign(&mut tx, &multisig_keys[1]);
        tx
    };
    assert_tx_success(tx_with_two_valid_signatures, &mut runner);

    // A transaction with three valid signatures should succeed in our 2/3 multisig
    let tx_with_three_valid_signatures = {
        let mut tx = generate_multisig_tx();
        sign(&mut tx, &multisig_keys[0]);
        sign(&mut tx, &multisig_keys[1]);
        sign(&mut tx, &multisig_keys[2]);
        tx
    };
    assert_tx_success(tx_with_three_valid_signatures, &mut runner);

    // A transaction with a different set of two valid signatures should succeed in our 2/3 multisig
    let tx_with_other_valid_signatures = {
        let mut tx = generate_multisig_tx();
        sign(&mut tx, &multisig_keys[1]);
        sign(&mut tx, &multisig_keys[2]);
        tx
    };
    assert_tx_success(tx_with_other_valid_signatures, &mut runner);

    // A transaction with a non-member signature should fail because it does not match the signed credential ID.
    let tx_with_non_member_signature = {
        let mut tx = generate_multisig_tx();
        sign(&mut tx, &multisig_keys[0]);
        sign(&mut tx, &multisig_keys[1]);
        // Manually add a signature from a non-member key
        {
            let non_member_key = TestPrivateKey::generate();
            let non_member_sig =
                tx.sign_without_adding(&non_member_key, &<RT as Runtime<S>>::CHAIN_HASH);
            tx.signatures
                .try_push(PubKeyAndSignature {
                    signature: non_member_sig,
                    pub_key: non_member_key.pub_key(),
                })
                .unwrap();
        }
        tx
    };
    assert_tx_skip(
        tx_with_non_member_signature,
        &mut runner,
        "Verification equation was not satisfied",
    );

    // A transaction with only one signature should be skipped because it does not meet the required threshold.
    let tx_with_too_few_signatures = {
        let mut tx = generate_multisig_tx();
        sign(&mut tx, &multisig_keys[0]);
        tx
    };
    assert_tx_skip(
        tx_with_too_few_signatures,
        &mut runner,
        "Not enough valid signatures. Required: 2, Got: 1",
    );

    // A transaction with a bad signature should fail verification
    let tx_with_bad_signature = {
        let mut tx = generate_multisig_tx();
        sign(&mut tx, &multisig_keys[0]);
        let bad_signature = multisig_keys[1].sign(&[1, 2, 3]);
        tx.add_signature(bad_signature, multisig_keys[1].pub_key())
            .unwrap();
        tx
    };
    assert_tx_skip(
        tx_with_bad_signature,
        &mut runner,
        "Verification equation was not satisfied",
    );

    // A transaction with a duplicate signature should be rejected
    let tx_with_duplicate_signature = {
        let mut tx = generate_multisig_tx();
        sign(&mut tx, &multisig_keys[0]);
        sign(&mut tx, &multisig_keys[1]);
        // Manually add a duplicate signature
        {
            let duplicate_sig =
                tx.sign_without_adding(&multisig_keys[0], &<RT as Runtime<S>>::CHAIN_HASH);
            tx.signatures
                .try_push(PubKeyAndSignature {
                    signature: duplicate_sig,
                    pub_key: multisig_keys[0].pub_key(),
                })
                .unwrap();
        }

        tx
    };

    // The duplicate key changes the credential_address in the signed bytes,
    // so signature verification fails.
    assert_tx_skip(
        tx_with_duplicate_signature,
        &mut runner,
        "Verification equation was not satisfied",
    );
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
        assert!(
            !accounts
                .is_authorized(
                    &non_registered_account.address(),
                    &non_registered_account.credential_id(),
                    state
                )
                .unwrap(),
            "unregistered account should have no authorization at genesis"
        );
    });

    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: non_registered_account.create_plain_message::<RT, Accounts<S>>(
            CallMessage::InsertCredentialId(new_credential),
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            let accounts = Accounts::<S>::default();

            // The new credential is authorized for the sender's address.
            assert!(
                accounts
                    .is_authorized(&non_registered_account.address(), &new_credential, state)
                    .unwrap(),
                "new credential should be authorized for the sender's address"
            );

            // The sender's own credential is auto-registered for the same
            // address when the tx flowed through `resolve_sender_address`.
            assert!(
                accounts
                    .is_authorized(
                        &non_registered_account.address(),
                        &non_registered_account.credential_id(),
                        state
                    )
                    .unwrap(),
                "the sender's own credential is auto-registered on first tx"
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

/// Genesis-registered credentials are authorized for their configured address
/// in `account_owners`. The resolver returns that address when queried with it
/// as the default; passing a different fallback auto-registers a new tuple for
/// the fallback (since the many-to-many model allows a credential to own
/// multiple addresses).
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

        // Resolving with account_1's registered address as the default returns
        // it directly (step 1 in the resolver).
        assert_eq!(
            accounts
                .resolve_sender_address(&account_1.address(), &account_1.credential_id(), state)
                .unwrap(),
            account_1.address()
        );

        // Resolving with a different default for the same credential returns
        // that different default (auto-registered); the many-to-many model
        // allows the credential to own both addresses.
        assert_eq!(
            accounts
                .resolve_sender_address(&account_2.address(), &account_1.credential_id(), state)
                .unwrap(),
            account_2.address()
        );
    });
}

/// After `InsertCredentialId` from a user, the new credential is authorized
/// for that user's address but its stateless default routing
/// (`credential_id.into()`) is unchanged. Resolving with the user's address as
/// default returns it; resolving with the credential's own default returns the
/// default (auto-registered).
#[test]
fn test_resolve_address_with_multi_credential_ownership() {
    let (
        TestData {
            non_registered_account,
            ..
        },
        mut runner,
    ) = setup();

    let credential_1 = TestPrivateKey::generate().pub_key().credential_id();
    let credential_2 = TestPrivateKey::generate().pub_key().credential_id();

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

        // With the user's address as default, both credentials route to it
        // (they were authorized via `InsertCredentialId`).
        assert_eq!(
            accounts
                .resolve_sender_address(&non_registered_account.address(), &credential_1, state)
                .unwrap(),
            non_registered_account.address()
        );
        assert_eq!(
            accounts
                .resolve_sender_address(&non_registered_account.address(), &credential_2, state)
                .unwrap(),
            non_registered_account.address()
        );

        // Both credentials are authorized under the user's address.
        assert!(accounts
            .is_authorized(&non_registered_account.address(), &credential_1, state)
            .unwrap());
        assert!(accounts
            .is_authorized(&non_registered_account.address(), &credential_2, state)
            .unwrap());
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

/// Verifies the PR 1 invariant: every write to the routing `accounts` map must
/// also write the matching `(address, credential_id)` tuple into
/// `account_owners`. This covers the three write paths (genesis,
/// `InsertCredentialId`, and auto-register on first resolve).
/// Verifies the PR 1 invariant: every write path (genesis, auto-register on
/// first resolve, and `InsertCredentialId`) lands its tuple in
/// `account_owners`. The legacy `accounts` map is never written post-freeze,
/// so these paths are the only sources of post-upgrade authorization state.
#[test]
fn test_account_owners_write_paths() {
    let (
        TestData {
            account_1,
            non_registered_account,
            ..
        },
        mut runner,
    ) = setup();

    // Genesis path.
    runner.query_visible_state(|state| {
        let accounts = Accounts::<S>::default();
        assert!(
            accounts
                .is_authorized(&account_1.address(), &account_1.credential_id(), state)
                .unwrap(),
            "genesis-registered credential should be present in account_owners"
        );
    });

    // Auto-register path: a `resolve_sender_address` call for an unseen
    // credential writes the tuple. `query_visible_state` scopes the write to
    // this closure so the check is in-session.
    runner.query_visible_state(|state| {
        let mut accounts = Accounts::<S>::default();
        let _ = accounts
            .resolve_sender_address(
                &non_registered_account.address(),
                &non_registered_account.credential_id(),
                state,
            )
            .unwrap();
        assert!(
            accounts
                .is_authorized(
                    &non_registered_account.address(),
                    &non_registered_account.credential_id(),
                    state
                )
                .unwrap(),
            "auto-registered credential should be present in account_owners"
        );
    });

    // InsertCredentialId path.
    let new_credential = TestPrivateKey::generate().pub_key().credential_id();
    runner.execute_transaction(TransactionTestCase {
        input: non_registered_account.create_plain_message::<RT, Accounts<S>>(
            CallMessage::InsertCredentialId(new_credential),
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_authorized(&non_registered_account.address(), &new_credential, state)
                    .unwrap(),
                "InsertCredentialId should populate account_owners under sender's address"
            );
        }),
    });
}

#[test]
fn test_disable_custom_account_mappings() {
    let (user, mut runner) = setup_with_disable_custom_account_mappings();

    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(
            new_credential,
        )),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "Custom account mappings are disabled"
                );
            }
            _ => panic!("Expected reverted transaction"),
        }),
    });
}
