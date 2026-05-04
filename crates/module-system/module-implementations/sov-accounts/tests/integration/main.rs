use sov_accounts::{AccountData, Accounts, CallMessage};
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
    // `account_2` is intentionally kept in the fixture for the upcoming
    // `target_address` PR. Until then it has no readers — silence the lint.
    #[allow(dead_code)]
    account_2: TestUser<S>,
    non_registered_account: TestUser<S>,
}

/// We set up genesis with three accounts, two of which have custom credentials
/// authorized at genesis.
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

    // The account is registered at genesis: its credential is authorized for
    // the user's address via `account_owners`.
    runner.query_visible_state(|state| {
        let accounts = Accounts::<S>::default();
        assert!(accounts
            .is_explicitly_authorized(&user.address(), &user.credential_id(), state)
            .unwrap());
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

            // The new credential is authorized for the user's address.
            assert!(accounts
                .is_explicitly_authorized(&user.address(), &new_credential, state)
                .unwrap());
            assert_ne!(new_credential, user.credential_id());
        }),
    });
}

/// A credential already authorized for an address cannot be inserted twice.
#[test]
fn test_insert_existing_credential_fails() {
    let (
        TestData {
            non_registered_account: sender,
            ..
        },
        mut runner,
    ) = setup();

    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: sender.create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(
            new_credential,
        )),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: sender.create_plain_message::<RT, Accounts<S>>(CallMessage::InsertCredentialId(
            new_credential,
        )),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "CredentialId already authorized for this address"
                );
            }
            _ => panic!("Expected reverted transaction for existing credential"),
        }),
    });
}

/// Tests the multisig functionality of the Accounts module.
///
/// Seeds genesis with a `TestUser` whose custom `credential_id` matches the
/// multisig, so the multisig's canonical address is funded before any tx is
/// submitted. This keeps the focus on signature-level invariants.
#[test]
fn test_setup_multisig_and_act() {
    use sov_modules_api::Multisig;

    let multisig_keys = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let multisig = Multisig::new(2, multisig_keys.iter().map(|k| k.pub_key()).collect());
    let multisig_credential_id =
        multisig.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();

    // Build a funded `TestUser` whose address is the multisig's canonical address.
    let multisig_user =
        TestUser::generate_with_default_balance().add_credential_id(multisig_credential_id);

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user.clone()]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

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
        .to_multisig_tx(multisig.clone(), None)
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

    assert_eq!(non_registered_account.custom_credential_id, None);

    runner.query_visible_state(|state| {
        let accounts = Accounts::<S>::default();
        assert!(!accounts
            .is_explicitly_authorized(
                &non_registered_account.address(),
                &non_registered_account.credential_id(),
                state
            )
            .unwrap());
        assert!(accounts
            .is_authorized_for(
                &non_registered_account.address(),
                &non_registered_account.credential_id(),
                state
            )
            .unwrap());
    });

    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: non_registered_account.create_plain_message::<RT, Accounts<S>>(
            CallMessage::InsertCredentialId(new_credential),
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());

            let accounts = Accounts::<S>::default();

            assert!(accounts
                .is_explicitly_authorized(&non_registered_account.address(), &new_credential, state)
                .unwrap());
            assert!(accounts
                .is_authorized_for(&non_registered_account.address(), &new_credential, state)
                .unwrap());

            assert!(!accounts
                .is_explicitly_authorized(
                    &non_registered_account.address(),
                    &non_registered_account.credential_id(),
                    state
                )
                .unwrap());
            assert!(accounts
                .is_authorized_for(
                    &non_registered_account.address(),
                    &non_registered_account.credential_id(),
                    state
                )
                .unwrap());

            assert_ne!(new_credential, non_registered_account.credential_id());
        }),
    });
}

/// After `InsertCredentialId` from a user, each inserted credential is
/// authorized under that user's address.
#[test]
fn test_authorize_multiple_credentials_for_same_address() {
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
        let accounts = Accounts::<S>::default();
        let addr = non_registered_account.address();

        assert!(accounts
            .is_explicitly_authorized(&addr, &credential_1, state)
            .unwrap());
        assert!(accounts
            .is_explicitly_authorized(&addr, &credential_2, state)
            .unwrap());
        assert!(accounts
            .is_authorized_for(&addr, &credential_1, state)
            .unwrap());
        assert!(accounts
            .is_authorized_for(&addr, &credential_2, state)
            .unwrap());
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

/// A multisig test fixture: keys, envelope, credential id, and a funded `TestUser`
/// registered at the multisig's default address in genesis.
struct MultisigEnv {
    keys: [TestPrivateKey; 3],
    multisig: sov_modules_api::Multisig<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey>,
    credential_id: sov_modules_api::CredentialId,
    user: TestUser<S>,
}

fn make_multisig_env() -> MultisigEnv {
    let keys = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let multisig = sov_modules_api::Multisig::new(2, keys.iter().map(|k| k.pub_key()).collect());
    let credential_id = multisig.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();
    let user = TestUser::generate_with_default_balance().add_credential_id(credential_id);
    MultisigEnv {
        keys,
        multisig,
        credential_id,
        user,
    }
}

/// Builds an unsigned V1 tx carrying an `InsertCredentialId` call.
fn make_v1_tx(
    multisig: &sov_modules_api::Multisig<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey>,
    inner_credential: sov_modules_api::CredentialId,
    target_address: Option<<S as Spec>::Address>,
) -> Version1<RT, S> {
    UnsignedTransactionV0::<RT, S>::new_with_details(
        TestAccountsRuntimeCall::Accounts(CallMessage::InsertCredentialId(inner_credential)),
        sov_modules_api::capabilities::UniquenessData::Generation(0),
        default_test_tx_details::<S>(),
    )
    .to_multisig_tx(multisig.clone(), target_address)
}

fn sign_v1(tx: &mut Version1<RT, S>, key: &TestPrivateKey) {
    tx.sign(key, &<RT as Runtime<S>>::CHAIN_HASH).unwrap();
}

fn submit_v1(tx: Version1<RT, S>) -> TransactionType<RT, S> {
    let tx = Transaction::<RT, S>::from(tx);
    TransactionType::<RT, S>::PreSigned(RawTx {
        data: borsh::to_vec(&tx).unwrap(),
    })
}

/// V1 tx with `target_address = None` routes through `resolve_sender_address`
/// and executes as the multisig's default address. Evidence: the `InsertCredentialId`
/// call writes `(multisig_default, inner_credential)` to `account_owners`.
#[test]
fn test_v1_target_none_uses_default_resolver() {
    let MultisigEnv {
        keys,
        multisig,
        user: multisig_user,
        ..
    } = make_multisig_env();
    let multisig_default_address = multisig_user.address();
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut tx = make_v1_tx(&multisig, inner_credential, None);
    sign_v1(&mut tx, &keys[0]);
    sign_v1(&mut tx, &keys[1]);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(tx),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_explicitly_authorized(&multisig_default_address, &inner_credential, state)
                    .unwrap(),
                "target=None should route to multisig default; the InsertCredentialId \
                 call must write the new credential under that address"
            );
        }),
    });
}

/// V1 tx with `target_address = Some(X)` where `(X, multisig_credential_id)` is
/// authorized resolves context as `X`. Evidence: `InsertCredentialId` writes
/// `(X, inner_credential)` — not `(multisig_default, inner_credential)`.
#[test]
fn test_v1_target_some_authorized_succeeds() {
    let MultisigEnv {
        keys,
        multisig,
        credential_id: multisig_credential_id,
        user: multisig_user,
    } = make_multisig_env();

    // Alice is a regular funded user; we authorize the multisig for her address.
    let alice = TestUser::<S>::generate_with_default_balance();
    let alice_address = alice.address();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user, alice]);
    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    genesis.accounts.accounts.push(AccountData {
        credential_id: multisig_credential_id,
        address: alice_address,
    });
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut tx = make_v1_tx(&multisig, inner_credential, Some(alice_address));
    sign_v1(&mut tx, &keys[0]);
    sign_v1(&mut tx, &keys[1]);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(tx),
        assert: Box::new(move |result, state| {
            assert!(
                result.tx_receipt.is_successful(),
                "V1 target=Some(authorized) should succeed, got {:?}",
                result.tx_receipt
            );
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_explicitly_authorized(&alice_address, &inner_credential, state)
                    .unwrap(),
                "InsertCredentialId should write under target_address, not multisig default"
            );
        }),
    });
}

/// V1 tx with `target_address = Some(Y)` where `(Y, credential_id) ∉ account_owners`
/// is skipped, not reverted. No state is mutated — in particular, no auto-register
/// on the `Some` path.
#[test]
fn test_v1_target_some_unauthorized_skipped() {
    let MultisigEnv {
        keys,
        multisig,
        user: multisig_user,
        ..
    } = make_multisig_env();
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    // Unowned: nothing in `account_owners` points to this address.
    let unowned_address = TestUser::<S>::generate_with_default_balance().address();
    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();

    let mut tx = make_v1_tx(&multisig, inner_credential, Some(unowned_address));
    sign_v1(&mut tx, &keys[0]);
    sign_v1(&mut tx, &keys[1]);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(tx),
        assert: Box::new(move |result, state| {
            match result.tx_receipt {
                TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                    let msg = error.to_string();
                    assert!(
                        msg.contains("not authorized for target address"),
                        "unexpected skip reason: {msg}"
                    );
                }
                other => panic!("expected skipped tx, got {other:?}"),
            }
            // No auto-register on the `Some` path: the unauthorized lookup must
            // not have created an entry.
            let accounts = Accounts::<S>::default();
            assert!(
                !accounts
                    .is_explicitly_authorized(&unowned_address, &inner_credential, state)
                    .unwrap(),
                "unauthorized target path must not auto-register any tuple"
            );
        }),
    });
}

/// `target_address` is part of the signed bytes: tampering with it after signing
/// invalidates the signature.
#[test]
fn test_v1_target_tamper_breaks_signature() {
    let MultisigEnv {
        keys,
        multisig,
        credential_id: multisig_credential_id,
        user: multisig_user,
    } = make_multisig_env();
    let alice = TestUser::<S>::generate_with_default_balance();
    let alice_address = alice.address();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user, alice]);
    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    // Authorize the multisig for both alice and a second, unrelated address so
    // the "tampered-to" address is also in the account_owners map. This isolates
    // the failure to signature verification rather than authorization.
    let tampered_address = TestUser::<S>::generate_with_default_balance().address();
    genesis.accounts.accounts.push(AccountData {
        credential_id: multisig_credential_id,
        address: alice_address,
    });
    genesis.accounts.accounts.push(AccountData {
        credential_id: multisig_credential_id,
        address: tampered_address,
    });
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut tx = make_v1_tx(&multisig, inner_credential, Some(alice_address));
    sign_v1(&mut tx, &keys[0]);
    sign_v1(&mut tx, &keys[1]);
    // Mutate after signing.
    tx.target_address = Some(tampered_address);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(tx),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                let msg = error.to_string();
                assert!(
                    msg.contains("Verification equation was not satisfied"),
                    "expected signature failure after target_address tamper, got: {msg}"
                );
            }
            other => panic!("expected skipped tx, got {other:?}"),
        }),
    });
}
