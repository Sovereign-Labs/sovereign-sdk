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
    account_2: TestUser<S>,
    non_registered_account: TestUser<S>,
}

/// We set up genesis with three accounts, two of which have custom credentials
/// authorized at genesis. Returns the genesis so callers can extend it
/// (e.g. push extra `account_owners` mappings) before building the runner.
fn setup_genesis() -> (TestData<S>, GenesisConfig<S>) {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![
        TestUser::generate_with_default_balance().add_credential_id([0u8; 32].into()),
        TestUser::generate_with_default_balance().add_credential_id([1u8; 32].into()),
        TestUser::generate_with_default_balance(),
    ]);

    let user_1 = genesis_config.additional_accounts()[0].clone();
    let user_2 = genesis_config.additional_accounts()[1].clone();
    let user_3 = genesis_config.additional_accounts()[2].clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());

    (
        TestData {
            account_1: user_1,
            account_2: user_2,
            non_registered_account: user_3,
        },
        genesis,
    )
}

fn setup() -> (TestData<S>, TestRunner<RT, S>) {
    let (data, genesis) = setup_genesis();
    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
    (data, runner)
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
            None,
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
    address_override: Option<<S as Spec>::Address>,
) -> Version1<RT, S> {
    let details = default_test_tx_details::<S>();
    UnsignedTransactionV0::<RT, S>::new(
        TestAccountsRuntimeCall::Accounts(CallMessage::InsertCredentialId(inner_credential)),
        details.chain_id,
        details.max_priority_fee_bips,
        details.max_fee,
        sov_modules_api::capabilities::UniquenessData::Generation(0),
        details.gas_limit,
        address_override,
    )
    .to_multisig_tx(multisig.clone())
}

/// Builds an unsigned V1 tx carrying the given accounts call.
fn make_v1_tx_with_call(
    multisig: &sov_modules_api::Multisig<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey>,
    call: CallMessage<S>,
    address_override: Option<<S as Spec>::Address>,
    generation: u64,
) -> Version1<RT, S> {
    UnsignedTransactionV0::<RT, S>::new_with_details(
        TestAccountsRuntimeCall::Accounts(call),
        sov_modules_api::capabilities::UniquenessData::Generation(generation),
        default_test_tx_details::<S>(),
        address_override,
    )
    .to_multisig_tx(multisig.clone())
}

fn make_v0_tx(
    sender: &TestUser<S>,
    inner_credential: sov_modules_api::CredentialId,
    address_override: Option<<S as Spec>::Address>,
) -> Transaction<RT, S> {
    let utx = UnsignedTransactionV0::<RT, S>::new_with_details(
        TestAccountsRuntimeCall::Accounts(CallMessage::InsertCredentialId(inner_credential)),
        sov_modules_api::capabilities::UniquenessData::Generation(0),
        default_test_tx_details::<S>(),
        address_override,
    );
    utx.sign(sender.private_key(), &<RT as Runtime<S>>::CHAIN_HASH)
}

fn sign_v1(tx: &mut Version1<RT, S>, key: &TestPrivateKey) {
    tx.sign(key, &<RT as Runtime<S>>::CHAIN_HASH).unwrap();
}

fn submit_v0(tx: Transaction<RT, S>) -> TransactionType<RT, S> {
    TransactionType::<RT, S>::PreSigned(RawTx {
        data: borsh::to_vec(&tx).unwrap(),
    })
}

fn submit_v1(tx: Version1<RT, S>) -> TransactionType<RT, S> {
    let tx = Transaction::<RT, S>::from(tx);
    TransactionType::<RT, S>::PreSigned(RawTx {
        data: borsh::to_vec(&tx).unwrap(),
    })
}

/// V1 tx with `address_override = None` executes as the multisig's default
/// address. Evidence: the `InsertCredentialId` call writes
/// `(multisig_default, inner_credential)` to `account_owners`.
#[test]
fn test_v1_address_override_none_uses_default_resolver() {
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
                "address_override=None should route to multisig default; the InsertCredentialId \
                 call must write the new credential under that address"
            );
        }),
    });
}

/// V1 tx with `address_override = Some(X)` where `(X, multisig_credential_id)` is
/// authorized resolves context as `X`. Evidence: `InsertCredentialId` writes
/// `(X, inner_credential)` — not `(multisig_default, inner_credential)`.
#[test]
fn test_v1_address_override_some_authorized_succeeds() {
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
                "V1 address_override=Some(authorized) should succeed, got {:?}",
                result.tx_receipt
            );
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_explicitly_authorized(&alice_address, &inner_credential, state)
                    .unwrap(),
                "InsertCredentialId should write under address_override, not multisig default"
            );
        }),
    });
}

/// V1 tx with `address_override = Some(Y)` where `(Y, credential_id) ∉ account_owners`
/// is skipped, not reverted. No state is mutated — in particular, no auto-register
/// on the `Some` path.
#[test]
fn test_v1_address_override_some_unauthorized_skipped() {
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
                        msg.contains("not authorized for address override"),
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
                "unauthorized address_override path must not auto-register any tuple"
            );
        }),
    });
}

/// `address_override = None` ignores any pre-existing `(X, multisig_credential_id)`
/// mapping and routes to the multisig's default address. The mapping is untouched.
#[test]
fn test_v1_address_override_none_ignores_existing_mapping() {
    let MultisigEnv {
        keys,
        multisig,
        credential_id: multisig_credential_id,
        user: multisig_user,
    } = make_multisig_env();
    let multisig_default_address = multisig_user.address();

    // Alice has an authorization for the multisig credential, but the V1 tx below
    // signs with `None` and so must not route through her address.
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
    let mut tx = make_v1_tx(&multisig, inner_credential, None);
    sign_v1(&mut tx, &keys[0]);
    sign_v1(&mut tx, &keys[1]);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(tx),
        assert: Box::new(move |result, state| {
            assert!(
                result.tx_receipt.is_successful(),
                "V1 address_override=None should succeed, got {:?}",
                result.tx_receipt
            );
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_explicitly_authorized(&multisig_default_address, &inner_credential, state)
                    .unwrap(),
                "None must route to multisig default"
            );
            assert!(
                !accounts
                    .is_explicitly_authorized(&alice_address, &inner_credential, state)
                    .unwrap(),
                "None must not write under alice's address"
            );
            assert!(
                accounts
                    .is_explicitly_authorized(&alice_address, &multisig_credential_id, state)
                    .unwrap(),
                "pre-existing (alice, multisig_credential_id) authorization must be preserved"
            );
        }),
    });
}

/// `address_override` is part of the signed bytes: tampering with it after signing
/// invalidates the signature.
#[test]
fn test_v1_address_override_tamper_breaks_signature() {
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
    tx.address_override = Some(tampered_address);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(tx),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                let msg = error.to_string();
                assert!(
                    msg.contains("Verification equation was not satisfied"),
                    "expected signature failure after address_override tamper, got: {msg}"
                );
            }
            other => panic!("expected skipped tx, got {other:?}"),
        }),
    });
}

/// V0 tx with `address_override = None` executes as the signer's default address.
#[test]
fn test_v0_address_override_none_uses_default_resolver() {
    let sender = TestUser::<S>::generate_with_default_balance();
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![sender.clone()]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();
    let tx = make_v0_tx(&sender, inner_credential, None);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v0(tx),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_explicitly_authorized(&sender.address(), &inner_credential, state)
                    .unwrap(),
                "address_override=None should route to the signer's default address"
            );
        }),
    });
}

/// V0 tx with `address_override = Some(X)` where `(X, credential_id)` is
/// authorized resolves context as `X`.
#[test]
fn test_v0_address_override_some_authorized_succeeds() {
    let (
        TestData {
            account_2,
            non_registered_account,
            ..
        },
        mut genesis,
    ) = setup_genesis();
    let alice_address = account_2.address();
    genesis.accounts.accounts.push(AccountData {
        credential_id: non_registered_account.credential_id(),
        address: alice_address,
    });
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();
    let tx = make_v0_tx(
        &non_registered_account,
        inner_credential,
        Some(alice_address),
    );

    runner.execute_transaction(TransactionTestCase {
        input: submit_v0(tx),
        assert: Box::new(move |result, state| {
            assert!(
                result.tx_receipt.is_successful(),
                "V0 address_override=Some(authorized) should succeed, got {:?}",
                result.tx_receipt
            );
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_explicitly_authorized(&alice_address, &inner_credential, state)
                    .unwrap(),
                "InsertCredentialId should write under address_override, not sender default"
            );
        }),
    });
}

/// V0 tx with `address_override = Some(Y)` where `(Y, credential_id) ∉ account_owners`
/// is skipped, not reverted.
#[test]
fn test_v0_address_override_some_unauthorized_skipped() {
    let sender = TestUser::<S>::generate_with_default_balance();
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![sender.clone()]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let unowned_address = TestUser::<S>::generate_with_default_balance().address();
    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();
    let tx = make_v0_tx(&sender, inner_credential, Some(unowned_address));

    runner.execute_transaction(TransactionTestCase {
        input: submit_v0(tx),
        assert: Box::new(move |result, state| {
            match result.tx_receipt {
                TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                    let msg = error.to_string();
                    assert!(
                        msg.contains("not authorized for address override"),
                        "unexpected skip reason: {msg}"
                    );
                }
                other => panic!("expected skipped tx, got {other:?}"),
            }
            let accounts = Accounts::<S>::default();
            assert!(
                !accounts
                    .is_explicitly_authorized(&unowned_address, &inner_credential, state)
                    .unwrap(),
                "unauthorized address_override path must not auto-register any tuple"
            );
        }),
    });
}

/// `address_override` is part of the signed bytes for V0 as well: tampering with it
/// after signing invalidates the signature.
#[test]
fn test_v0_address_override_tamper_breaks_signature() {
    let sender = TestUser::<S>::generate_with_default_balance();
    let alice = TestUser::<S>::generate_with_default_balance();
    let alice_address = alice.address();
    let tampered_address = TestUser::<S>::generate_with_default_balance().address();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![sender.clone(), alice]);
    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    genesis.accounts.accounts.push(AccountData {
        credential_id: sender.credential_id(),
        address: alice_address,
    });
    genesis.accounts.accounts.push(AccountData {
        credential_id: sender.credential_id(),
        address: tampered_address,
    });
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let inner_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut tx = make_v0_tx(&sender, inner_credential, Some(alice_address));
    let Transaction::V0(inner) = &mut tx else {
        panic!("expected v0 tx");
    };
    inner.address_override = Some(tampered_address);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v0(tx),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                let msg = error.to_string();
                assert!(
                    msg.contains("Verification equation was not satisfied"),
                    "expected signature failure after address_override tamper, got: {msg}"
                );
            }
            other => panic!("expected skipped tx, got {other:?}"),
        }),
    });
}

/// An existing owner can authorize a second credential for their own address
/// via `AddCredentialToAddress` (V0 path, sender == address).
#[test]
fn test_add_credential_to_address_by_owner() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential: new_credential,
        }),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(
                accounts
                    .is_authorized_for(&owner_address, &new_credential, state)
                    .unwrap(),
                "new credential should be authorized for owner's address"
            );
        }),
    });
}

/// A brand-new user (no genesis entry, no prior `account_owners` entry) can
/// authorize a credential for their own default address. The simple
/// `sender == address` rule subsumes the bootstrap case: `sender` is the
/// user's default address because the resolver returns the default for an
/// unknown credential.
#[test]
fn test_add_credential_to_address_bootstrap() {
    let (
        TestData {
            non_registered_account: fresh_user,
            ..
        },
        mut runner,
    ) = setup();
    let fresh_address = fresh_user.address();
    let extra_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: fresh_user.create_plain_message::<RT, Accounts<S>>(
            CallMessage::AddCredentialToAddress {
                address: fresh_address,
                credential: extra_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(accounts
                .is_authorized_for(&fresh_address, &extra_credential, state)
                .unwrap());
        }),
    });
}

/// A caller cannot add a credential for an address that isn't their own.
/// Under the resolver, `context.sender()` is the caller's resolved address;
/// the handler rejects if `message.address != context.sender()`. Uses two
/// users whose canonical (`hash(pubkey).into()`) addresses equal their funded
/// addresses so V0 gas reservation succeeds and the handler actually runs.
#[test]
fn test_add_credential_to_address_non_owner_rejected() {
    let attacker = TestUser::<S>::generate_with_default_balance();
    let victim = TestUser::<S>::generate_with_default_balance();
    let victim_address = victim.address();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![attacker.clone(), victim]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let attacker_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: attacker.create_plain_message::<RT, Accounts<S>>(
            CallMessage::AddCredentialToAddress {
                address: victim_address,
                credential: attacker_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            match result.tx_receipt {
                TxEffect::Reverted(contents) => {
                    assert_eq!(
                        contents.reason.to_string(),
                        "Caller is not authorized to modify credentials for this address"
                    );
                }
                other => panic!("Expected reverted transaction, got {other:?}"),
            }
            // Confirm the attacker's credential was never written under the victim.
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&victim_address, &attacker_credential, state)
                .unwrap());
        }),
    });
}

/// Adding a tuple that already exists fails cleanly.
#[test]
fn test_add_credential_duplicate_rejected() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute(owner.create_plain_message::<RT, Accounts<S>>(
        CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential,
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential,
        }),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "CredentialId already authorized for this address"
                );
            }
            other => panic!("Expected reverted transaction, got {other:?}"),
        }),
    });
}

/// The owner can revoke a credential from their own address.
#[test]
fn test_remove_credential_from_address_by_owner() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let kept_credential = TestPrivateKey::generate().pub_key().credential_id();
    let removed_credential = TestPrivateKey::generate().pub_key().credential_id();

    // Authorize both credentials for the owner.
    runner.execute(owner.create_plain_message::<RT, Accounts<S>>(
        CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential: kept_credential,
        },
    ));
    runner.execute(owner.create_plain_message::<RT, Accounts<S>>(
        CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential: removed_credential,
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RemoveCredentialFromAddress {
                address: owner_address,
                credential: removed_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&owner_address, &removed_credential, state)
                .unwrap());
            assert!(
                accounts
                    .is_authorized_for(&owner_address, &kept_credential, state)
                    .unwrap(),
                "unrelated authorization should survive the revocation"
            );
        }),
    });
}

/// Removing a credential from its own canonical address writes an explicit
/// denial, because there is no legacy map entry to delete for that fallback.
#[test]
fn test_remove_canonical_credential_blocks_fallback_authorization() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let owner_credential = owner.credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RemoveCredentialFromAddress {
                address: owner_address,
                credential: owner_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&owner_address, &owner_credential, state)
                .unwrap());
        }),
    });
}

/// A caller cannot revoke a credential from an address they don't own. Uses
/// two canonical-address users (no custom credential_id) so the V0 gas path
/// resolves correctly to the funded address.
#[test]
fn test_remove_credential_non_owner_rejected() {
    let attacker = TestUser::<S>::generate_with_default_balance();
    let victim = TestUser::<S>::generate_with_default_balance();
    let victim_address = victim.address();
    let victim_credential = victim.credential_id();

    // Seed victim's `(address, credential)` so the attacker's revoke attempt
    // targets a real tuple. Otherwise the handler would fail on the tuple
    // check before the ownership check, hiding the bug we care about.
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![attacker.clone(), victim]);
    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    genesis.accounts.accounts.push(AccountData {
        credential_id: victim_credential,
        address: victim_address,
    });
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    runner.execute_transaction(TransactionTestCase {
        input: attacker.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RemoveCredentialFromAddress {
                address: victim_address,
                credential: victim_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            match result.tx_receipt {
                TxEffect::Reverted(contents) => {
                    assert_eq!(
                        contents.reason.to_string(),
                        "Caller is not authorized to modify credentials for this address"
                    );
                }
                other => panic!("Expected reverted transaction, got {other:?}"),
            }
            // Victim's authorization is unaffected.
            let accounts = Accounts::<S>::default();
            assert!(accounts
                .is_authorized_for(&victim_address, &victim_credential, state)
                .unwrap());
        }),
    });
}

/// Attempting to remove a tuple that doesn't exist fails cleanly.
#[test]
fn test_remove_credential_not_authorized_rejected() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let ghost_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RemoveCredentialFromAddress {
                address: owner_address,
                credential: ghost_credential,
            },
        ),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "CredentialId is not authorized for this address"
                );
            }
            other => panic!("Expected reverted transaction, got {other:?}"),
        }),
    });
}

/// Removing the last credential succeeds (no orphan guard) and leaves the
/// address unspendable via `account_owners`: a subsequent V1 tx with
/// `target_address = Some(orphaned)` signed by the revoked credential is
/// skipped at the resolver.
#[test]
fn test_remove_last_credential_orphans_address() {
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
    // Seed `(alice_address, multisig_credential_id)` so the multisig can route
    // to Alice's address before we revoke.
    genesis.accounts.accounts.push(AccountData {
        credential_id: multisig_credential_id,
        address: alice_address,
    });
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    // Revoke the only credential authorizing the multisig for Alice.
    let mut revoke_tx = make_v1_tx_with_call(
        &multisig,
        CallMessage::RemoveCredentialFromAddress {
            address: alice_address,
            credential: multisig_credential_id,
        },
        Some(alice_address),
        0,
    );
    sign_v1(&mut revoke_tx, &keys[0]);
    sign_v1(&mut revoke_tx, &keys[1]);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(revoke_tx),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&alice_address, &multisig_credential_id, state)
                .unwrap());
        }),
    });

    // Orphan confirmed: trying to route to Alice again is rejected at the
    // resolver (skipped, not reverted).
    let follow_up_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut orphan_tx = make_v1_tx_with_call(
        &multisig,
        CallMessage::InsertCredentialId(follow_up_credential),
        Some(alice_address),
        1,
    );
    sign_v1(&mut orphan_tx, &keys[0]);
    sign_v1(&mut orphan_tx, &keys[1]);

    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(orphan_tx),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                let msg = error.to_string();
                assert!(
                    msg.contains("not authorized for address override"),
                    "expected resolver skip; got: {msg}"
                );
            }
            other => panic!("expected skipped tx, got {other:?}"),
        }),
    });
}

/// End-to-end multisig rotation M1 → M2 while preserving the original
/// address. Exercises `AddCredentialToAddress` + `RemoveCredentialFromAddress`
/// via V1 + `target_address`.
#[test]
fn test_multisig_key_rotation() {
    use sov_modules_api::Multisig;

    // M1 is the incumbent 2-of-3 multisig. M2 is the rotated key set, fully
    // disjoint so credential_id_1 and credential_id_2 cannot collide.
    let keys_1 = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let keys_2 = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let multisig_1 = Multisig::new(2, keys_1.iter().map(|k| k.pub_key()).collect());
    let multisig_2 = Multisig::new(2, keys_2.iter().map(|k| k.pub_key()).collect());
    let credential_id_1 =
        multisig_1.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();
    let credential_id_2 =
        multisig_2.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();

    let multisig_user =
        TestUser::generate_with_default_balance().add_credential_id(credential_id_1);
    let target_address = multisig_user.address();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    // 1. M1 authorizes M2 for the shared address X.
    let mut add_tx = make_v1_tx_with_call(
        &multisig_1,
        CallMessage::AddCredentialToAddress {
            address: target_address,
            credential: credential_id_2,
        },
        Some(target_address),
        0,
    );
    sign_v1(&mut add_tx, &keys_1[0]);
    sign_v1(&mut add_tx, &keys_1[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(add_tx),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(accounts
                .is_authorized_for(&target_address, &credential_id_1, state)
                .unwrap());
            assert!(accounts
                .is_authorized_for(&target_address, &credential_id_2, state)
                .unwrap());
        }),
    });

    // 2. M1 revokes itself from X.
    let mut remove_tx = make_v1_tx_with_call(
        &multisig_1,
        CallMessage::RemoveCredentialFromAddress {
            address: target_address,
            credential: credential_id_1,
        },
        Some(target_address),
        1,
    );
    sign_v1(&mut remove_tx, &keys_1[0]);
    sign_v1(&mut remove_tx, &keys_1[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(remove_tx),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&target_address, &credential_id_1, state)
                .unwrap());
            assert!(accounts
                .is_authorized_for(&target_address, &credential_id_2, state)
                .unwrap());
        }),
    });

    // 3. M2 signs a routine tx targeting X — should succeed.
    let follow_up_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut m2_tx = make_v1_tx_with_call(
        &multisig_2,
        CallMessage::InsertCredentialId(follow_up_credential),
        Some(target_address),
        0,
    );
    sign_v1(&mut m2_tx, &keys_2[0]);
    sign_v1(&mut m2_tx, &keys_2[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(m2_tx),
        assert: Box::new(move |result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "M2 should now control X; got {:?}",
                result.tx_receipt
            );
        }),
    });

    // 4. M1 signs another tx targeting X — should be skipped (no longer
    //    authorized).
    let orphan_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut m1_tx = make_v1_tx_with_call(
        &multisig_1,
        CallMessage::InsertCredentialId(orphan_credential),
        Some(target_address),
        2,
    );
    sign_v1(&mut m1_tx, &keys_1[0]);
    sign_v1(&mut m1_tx, &keys_1[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(m1_tx),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                let msg = error.to_string();
                assert!(
                    msg.contains("not authorized for address override"),
                    "expected resolver skip after rotation; got: {msg}"
                );
            }
            other => panic!("expected skipped tx after rotation, got {other:?}"),
        }),
    });
}

/// With `enable_custom_account_mappings = false`, both new call variants are
/// rejected by the same guard that already rejects `InsertCredentialId`.
#[test]
fn test_feature_flag_gates_new_variants() {
    let (user, mut runner) = setup_with_disable_custom_account_mappings();
    let user_address = user.address();
    let credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Accounts<S>>(CallMessage::AddCredentialToAddress {
            address: user_address,
            credential,
        }),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "Custom account mappings are disabled"
                );
            }
            other => panic!("Expected reverted transaction, got {other:?}"),
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RemoveCredentialFromAddress {
                address: user_address,
                credential,
            },
        ),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "Custom account mappings are disabled"
                );
            }
            other => panic!("Expected reverted transaction, got {other:?}"),
        }),
    });
}

/// `RotateCredentialOnAddress` atomically replaces an authorized credential
/// with a new one. The old tuple is deleted; the new tuple is written; a
/// separate unrelated authorization on the same address is untouched.
#[test]
fn test_rotate_credential_by_owner() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let old_credential = TestPrivateKey::generate().pub_key().credential_id();
    let new_credential = TestPrivateKey::generate().pub_key().credential_id();
    let unrelated_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute(owner.create_plain_message::<RT, Accounts<S>>(
        CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential: old_credential,
        },
    ));
    runner.execute(owner.create_plain_message::<RT, Accounts<S>>(
        CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential: unrelated_credential,
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RotateCredentialOnAddress {
                address: owner_address,
                old_credential,
                new_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&owner_address, &old_credential, state)
                .unwrap());
            assert!(accounts
                .is_authorized_for(&owner_address, &new_credential, state)
                .unwrap());
            assert!(
                accounts
                    .is_authorized_for(&owner_address, &unrelated_credential, state)
                    .unwrap(),
                "unrelated authorization must survive rotation"
            );
        }),
    });
}

/// Attempting to rotate out a credential that isn't authorized fails and
/// leaves state untouched.
#[test]
fn test_rotate_credential_old_not_authorized_rejected() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let ghost_credential = TestPrivateKey::generate().pub_key().credential_id();
    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RotateCredentialOnAddress {
                address: owner_address,
                old_credential: ghost_credential,
                new_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            match result.tx_receipt {
                TxEffect::Reverted(contents) => {
                    assert_eq!(
                        contents.reason.to_string(),
                        "CredentialId is not authorized for this address"
                    );
                }
                other => panic!("Expected reverted transaction, got {other:?}"),
            }
            // The revert must leave the new tuple unwritten.
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&owner_address, &new_credential, state)
                .unwrap());
        }),
    });
}

/// Attempting to rotate in a credential that is already authorized fails
/// without revoking the old credential.
#[test]
fn test_rotate_credential_new_already_authorized_rejected() {
    let (
        TestData {
            non_registered_account: owner,
            ..
        },
        mut runner,
    ) = setup();
    let owner_address = owner.address();
    let old_credential = TestPrivateKey::generate().pub_key().credential_id();
    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute(owner.create_plain_message::<RT, Accounts<S>>(
        CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential: old_credential,
        },
    ));
    runner.execute(owner.create_plain_message::<RT, Accounts<S>>(
        CallMessage::AddCredentialToAddress {
            address: owner_address,
            credential: new_credential,
        },
    ));

    runner.execute_transaction(TransactionTestCase {
        input: owner.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RotateCredentialOnAddress {
                address: owner_address,
                old_credential,
                new_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            match result.tx_receipt {
                TxEffect::Reverted(contents) => {
                    assert_eq!(
                        contents.reason.to_string(),
                        "CredentialId already authorized for this address"
                    );
                }
                other => panic!("Expected reverted transaction, got {other:?}"),
            }
            // The old credential must still be authorized — the revert rolls
            // back the delete.
            let accounts = Accounts::<S>::default();
            assert!(accounts
                .is_authorized_for(&owner_address, &old_credential, state)
                .unwrap());
            assert!(accounts
                .is_authorized_for(&owner_address, &new_credential, state)
                .unwrap());
        }),
    });
}

/// A caller cannot rotate credentials on an address they don't own.
#[test]
fn test_rotate_credential_non_owner_rejected() {
    let attacker = TestUser::<S>::generate_with_default_balance();
    let victim = TestUser::<S>::generate_with_default_balance();
    let victim_address = victim.address();
    let victim_credential = victim.credential_id();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![attacker.clone(), victim]);
    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    genesis.accounts.accounts.push(AccountData {
        credential_id: victim_credential,
        address: victim_address,
    });
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let attacker_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: attacker.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RotateCredentialOnAddress {
                address: victim_address,
                old_credential: victim_credential,
                new_credential: attacker_credential,
            },
        ),
        assert: Box::new(move |result, state| {
            match result.tx_receipt {
                TxEffect::Reverted(contents) => {
                    assert_eq!(
                        contents.reason.to_string(),
                        "Caller is not authorized to modify credentials for this address"
                    );
                }
                other => panic!("Expected reverted transaction, got {other:?}"),
            }
            let accounts = Accounts::<S>::default();
            assert!(accounts
                .is_authorized_for(&victim_address, &victim_credential, state)
                .unwrap());
            assert!(!accounts
                .is_authorized_for(&victim_address, &attacker_credential, state)
                .unwrap());
        }),
    });
}

/// `RotateCredentialOnAddress` is gated by the same feature flag as the
/// other write variants.
#[test]
fn test_rotate_credential_feature_flag_gated() {
    let (user, mut runner) = setup_with_disable_custom_account_mappings();
    let user_address = user.address();
    let old_credential = TestPrivateKey::generate().pub_key().credential_id();
    let new_credential = TestPrivateKey::generate().pub_key().credential_id();

    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Accounts<S>>(
            CallMessage::RotateCredentialOnAddress {
                address: user_address,
                old_credential,
                new_credential,
            },
        ),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Reverted(contents) => {
                assert_eq!(
                    contents.reason.to_string(),
                    "Custom account mappings are disabled"
                );
            }
            other => panic!("Expected reverted transaction, got {other:?}"),
        }),
    });
}

/// Multisig rotation M1 → M2 in a single atomic `RotateCredentialOnAddress`
/// call, collapsing the two-step Add+Remove dance.
#[test]
fn test_multisig_key_rotation_atomic() {
    use sov_modules_api::Multisig;

    let keys_1 = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let keys_2 = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let multisig_1 = Multisig::new(2, keys_1.iter().map(|k| k.pub_key()).collect());
    let multisig_2 = Multisig::new(2, keys_2.iter().map(|k| k.pub_key()).collect());
    let credential_id_1 =
        multisig_1.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();
    let credential_id_2 =
        multisig_2.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();

    let multisig_user =
        TestUser::generate_with_default_balance().add_credential_id(credential_id_1);
    let target_address = multisig_user.address();

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts(vec![multisig_user]);
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());

    let mut rotate_tx = make_v1_tx_with_call(
        &multisig_1,
        CallMessage::RotateCredentialOnAddress {
            address: target_address,
            old_credential: credential_id_1,
            new_credential: credential_id_2,
        },
        Some(target_address),
        0,
    );
    sign_v1(&mut rotate_tx, &keys_1[0]);
    sign_v1(&mut rotate_tx, &keys_1[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(rotate_tx),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let accounts = Accounts::<S>::default();
            assert!(!accounts
                .is_authorized_for(&target_address, &credential_id_1, state)
                .unwrap());
            assert!(accounts
                .is_authorized_for(&target_address, &credential_id_2, state)
                .unwrap());
        }),
    });

    // M1 cannot bypass the revocation by omitting target_address and falling
    // back to its canonical address.
    let stale_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut m1_target_none_tx = make_v1_tx_with_call(
        &multisig_1,
        CallMessage::InsertCredentialId(stale_credential),
        None,
        1,
    );
    sign_v1(&mut m1_target_none_tx, &keys_1[0]);
    sign_v1(&mut m1_target_none_tx, &keys_1[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(m1_target_none_tx),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                let msg = error.to_string();
                assert!(
                    msg.contains("not authorized for resolved address"),
                    "expected resolver skip after target-less rotation fallback; got: {msg}"
                );
            }
            other => panic!("expected skipped tx after target-less fallback, got {other:?}"),
        }),
    });

    // M2 now controls X.
    let follow_up_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut m2_tx = make_v1_tx_with_call(
        &multisig_2,
        CallMessage::InsertCredentialId(follow_up_credential),
        Some(target_address),
        0,
    );
    sign_v1(&mut m2_tx, &keys_2[0]);
    sign_v1(&mut m2_tx, &keys_2[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(m2_tx),
        assert: Box::new(move |result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "M2 should now control X after atomic rotation; got {:?}",
                result.tx_receipt
            );
        }),
    });

    // M1 can no longer act as X.
    let orphan_credential = TestPrivateKey::generate().pub_key().credential_id();
    let mut m1_tx = make_v1_tx_with_call(
        &multisig_1,
        CallMessage::InsertCredentialId(orphan_credential),
        Some(target_address),
        1,
    );
    sign_v1(&mut m1_tx, &keys_1[0]);
    sign_v1(&mut m1_tx, &keys_1[1]);
    runner.execute_transaction(TransactionTestCase {
        input: submit_v1(m1_tx),
        assert: Box::new(move |result, _state| match result.tx_receipt {
            TxEffect::Skipped(SkippedTxContents { error, .. }) => {
                let msg = error.to_string();
                assert!(
                    msg.contains("not authorized for address override"),
                    "expected resolver skip after rotation; got: {msg}"
                );
            }
            other => panic!("expected skipped tx after rotation, got {other:?}"),
        }),
    });
}
