use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_api::{
    CredentialId, CryptoSpec, EncodeCall, HexHash, Multisig, PrivateKey, RawTx, Runtime, Spec,
    TxEffect,
};
use sov_test_utils::{
    default_test_tx_details, TestPrivateKey, TestUser, TransactionTestCase, TransactionType,
    TxProcessingError,
};
use sov_uniqueness::{Uniqueness, Window};
use sov_value_setter::ValueSetter;

use crate::runtime::{RT, S};
use crate::utils::{generate_default_tx, setup, setup_with_admin};

#[test]
fn send_tx_works_nonce() {
    let (admin, mut runner, evm_account) = setup();
    let evm_credential_id = CredentialId(HexHash::new(evm_account.address().into_word().into()));

    runner.query_visible_state(|state| {
        assert_eq!(
            Uniqueness::<S>::default()
                .nonce(&evm_credential_id, state)
                .unwrap_infallible(),
            None,
            "The nonce should not be set"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Nonce(0), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());

            assert_eq!(
                Uniqueness::<S>::default()
                    .nonce(&evm_credential_id, state)
                    .unwrap_infallible(),
                Some(1),
                "The nonce should be 1"
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Nonce(1), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(
                Uniqueness::<S>::default()
                    .nonce(&evm_credential_id, state)
                    .unwrap_infallible(),
                Some(2),
                "The nonce should be 2"
            );
        }),
    });
}

#[test]
fn send_tx_works_generation() {
    let (admin, mut runner, evm_account) = setup();
    let admin_credential_id: CredentialId = admin.credential_id();

    runner.query_visible_state(|state| {
        assert_eq!(
            Uniqueness::<S>::default()
                .next_generation(&admin_credential_id, state)
                .unwrap(),
            0,
            "The next generation for a new account should start at 0"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Generation(0), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());

            assert_eq!(
                Uniqueness::<S>::default()
                    .next_generation(&admin_credential_id, state)
                    .unwrap(),
                1,
                "The next generation should be 1 after a transaction of generation 0 is sent"
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Generation(5), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(
                Uniqueness::<S>::default()
                    .next_generation(&admin_credential_id, state)
                    .unwrap(),
                6,
                "The next available generation should update when a transaction with a higher generation is sent"
            );
        }),
    });
}

#[test]
fn send_tx_bad_generation_duplicate() {
    let (admin, mut runner, evm_account) = setup();

    // initialise generation
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Generation(5), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            assert!(ctx.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Generation(5), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            if let TxEffect::Skipped(skipped) = &ctx.tx_receipt {
                assert!(matches!(
                    skipped.error,
                    TxProcessingError::CheckUniquenessFailed(_)
                ));
            } else {
                panic!(
                    "Expected Skipped error, but got a different TxEffect: {:?}",
                    ctx.tx_receipt
                );
            }
        }),
    });
}

#[test]
fn send_tx_bad_generation_duplicate_with_malleated_v1_envelope() {
    let multisig_keys = [
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];
    let runtime_msg =
        <RT as EncodeCall<ValueSetter<S>>>::to_decodable(sov_value_setter::CallMessage::SetValue {
            value: 10,
            gas: None,
        });
    let multisig = Multisig::new(2, multisig_keys.iter().map(|key| key.pub_key()).collect());
    let multisig_credential_id =
        multisig.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();
    let (_, mut runner, _) = setup_with_admin(
        TestUser::<S>::generate_with_default_balance().add_credential_id(multisig_credential_id),
    );

    let mut original_tx = UnsignedTransaction::<RT, S>::new_with_details(
        runtime_msg,
        UniquenessData::Generation(0),
        default_test_tx_details::<S>(),
        None,
    )
    .to_multisig_tx(multisig);
    original_tx
        .sign(&multisig_keys[0], &RT::CHAIN_HASH)
        .unwrap();
    original_tx
        .sign(&multisig_keys[1], &RT::CHAIN_HASH)
        .unwrap();

    let mut malleated_tx = original_tx.clone();
    malleated_tx.unused_pub_keys.swap(0, 1);

    assert_eq!(
        original_tx.signing_bytes(&RT::CHAIN_HASH),
        malleated_tx.signing_bytes(&RT::CHAIN_HASH),
        "The signable payload should be unchanged by V1 envelope malleation"
    );
    assert_ne!(
        Transaction::<RT, S>::from(original_tx.clone()).hash(),
        Transaction::<RT, S>::from(malleated_tx.clone()).hash(),
        "The raw transaction hash should change when the V1 envelope is malleated"
    );

    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::PreSigned(RawTx {
            data: borsh::to_vec(&Transaction::<RT, S>::from(original_tx)).unwrap(),
        }),
        assert: Box::new(move |ctx, _state| {
            assert!(ctx.tx_receipt.is_successful(), "{:?}", ctx.tx_receipt);
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::PreSigned(RawTx {
            data: borsh::to_vec(&Transaction::<RT, S>::from(malleated_tx)).unwrap(),
        }),
        assert: Box::new(move |ctx, _state| {
            let TxEffect::Skipped(skipped) = &ctx.tx_receipt else {
                panic!(
                    "Expected Skipped error from uniqueness check, got {:?}",
                    ctx.tx_receipt
                );
            };

            match &skipped.error {
                TxProcessingError::CheckUniquenessFailed(reason) => {
                    assert!(reason.contains("Duplicate transaction"));
                }
                _ => {
                    panic!(
                        "Expected uniqueness rejection, got a different error: {:?}",
                        skipped.error
                    );
                }
            }
        }),
    });
}

#[test]
fn send_tx_bad_generation_too_old() {
    let (admin, mut runner, evm_account) = setup();

    // initialise generation
    let generation = config_value!("PAST_TRANSACTION_GENERATIONS");
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Generation(generation), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            assert!(ctx.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Generation(0), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            if let TxEffect::Skipped(skipped) = &ctx.tx_receipt {
                assert!(matches!(
                    skipped.error,
                    TxProcessingError::CheckUniquenessFailed(_)
                ));
            } else {
                panic!(
                    "Expected Skipped error, but got a different TxEffect: {:?}",
                    ctx.tx_receipt
                );
            }
        }),
    });
}

#[test]
fn send_tx_works_window_zero_start_nonce() {
    let (admin, mut runner, evm_account) = setup();
    let admin_credential_id: CredentialId = admin.credential_id();

    runner.query_visible_state(|state| {
        assert_eq!(
            Uniqueness::<S>::default()
                .window(&admin_credential_id, state)
                .unwrap_infallible(),
            None,
            "The window should not be set"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(0), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());

            assert_eq!(
                Uniqueness::<S>::default()
                    .window(&admin_credential_id, state)
                    .unwrap_infallible(),
                Some(Window::test_only_from_tuple((0, vec![1]))),
                "A bit in the window should be set",
            );
        }),
    });
}

#[test]
fn send_tx_works_window() {
    let (admin, mut runner, evm_account) = setup();
    let admin_credential_id: CredentialId = admin.credential_id();

    runner.query_visible_state(|state| {
        assert_eq!(
            Uniqueness::<S>::default()
                .window(&admin_credential_id, state)
                .unwrap_infallible(),
            None,
            "The window should not be set"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(42), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());

            assert_eq!(
                Uniqueness::<S>::default()
                    .window(&admin_credential_id, state)
                    .unwrap_infallible(),
                Some(Window::test_only_from_tuple((
                    0,
                    vec![0, 0, 0, 0, 0, 1 << 2]
                ))),
                "A bit in the window should be set",
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(40), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            assert_eq!(
                Uniqueness::<S>::default()
                    .window(&admin_credential_id, state)
                    .unwrap_infallible(),
                Some(Window::test_only_from_tuple((
                    0,
                    vec![0, 0, 0, 0, 0, (1 << 2) | (1 << 0)]
                ))),
                "Two bits in the window should be set."
            );
        }),
    });
    let window = config_value!("PAST_TRANSACTIONS_WINDOW");
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(
            UniquenessData::Window(40 + window - 1),
            &admin,
            &evm_account,
        ),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            let mut dst = vec![0; window as usize / 8];
            dst[0] = (1 << 2) | (1 << 0);
            dst[window as usize / 8 - 1] = 1 << 7;
            assert_eq!(
                Uniqueness::<S>::default()
                    .window(&admin_credential_id, state)
                    .unwrap_infallible(),
                Some(Window::test_only_from_tuple((40, dst))),
                "Dropping bits in the window"
            );
        }),
    });
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(40 + window), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());
            let mut dst = vec![0; window as usize / 8];
            dst[window as usize / 8 - 2] = 1 << 7;
            dst[window as usize / 8 - 1] = 1 << 0;
            assert_eq!(
                Uniqueness::<S>::default()
                    .window(&admin_credential_id, state)
                    .unwrap_infallible(),
                Some(Window::test_only_from_tuple((48, dst))),
                "Dropping bits in the window"
            );
        }),
    });
}

#[test]
fn send_tx_bad_window_duplicate() {
    let (admin, mut runner, evm_account) = setup();

    let nonce = config_value!("PAST_TRANSACTIONS_WINDOW") * 4 + 42;
    // initialise generation
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(nonce), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            assert!(ctx.tx_receipt.is_successful());
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(nonce), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            if let TxEffect::Skipped(skipped) = &ctx.tx_receipt {
                assert!(matches!(
                    skipped.error,
                    TxProcessingError::CheckUniquenessFailed(_)
                ));
            } else {
                panic!(
                    "Expected Skipped error, but got a different TxEffect: {:?}",
                    ctx.tx_receipt
                );
            }
        }),
    });
}

#[test]
fn send_tx_bad_window_nonce_too_old() {
    let (admin, mut runner, evm_account) = setup();
    let admin_credential_id: CredentialId = admin.credential_id();
    let window = config_value!("PAST_TRANSACTIONS_WINDOW");

    let mut dst = vec![0; window as usize / 8];
    dst[window as usize / 8 - 1] = 1;

    let value = dst.clone();
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(window), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful());

            assert_eq!(
                Uniqueness::<S>::default()
                    .window(&admin_credential_id, state)
                    .unwrap_infallible(),
                Some(Window::test_only_from_tuple((8, value))),
                "Dropping bits in the window"
            );
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(7), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert_eq!(
                Uniqueness::<S>::default()
                    .window(&admin_credential_id, state)
                    .unwrap_infallible(),
                Some(Window::test_only_from_tuple((8, dst))),
                "Unchanged window",
            );
            if let TxEffect::Skipped(skipped) = &ctx.tx_receipt {
                assert!(matches!(
                    skipped.error,
                    TxProcessingError::CheckUniquenessFailed(_)
                ));
            } else {
                panic!(
                    "Expected Skipped error, but got a different TxEffect: {:?}",
                    ctx.tx_receipt
                );
            }
        }),
    });
}
