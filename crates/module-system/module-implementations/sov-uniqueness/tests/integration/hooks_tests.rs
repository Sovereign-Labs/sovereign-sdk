use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{CredentialId, HexHash, TxEffect};
use sov_test_utils::{TransactionTestCase, TxProcessingError};
use sov_uniqueness::Uniqueness;

use crate::runtime::S;
use crate::utils::{generate_default_tx, setup};

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
fn send_tx_bad_nonce() {
    let (admin, mut runner, evm_account) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Nonce(5), &admin, &evm_account),
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
fn send_tx_bad_generation_too_old() {
    let (admin, mut runner, evm_account) = setup();

    // initialise generation
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Generation(10), &admin, &evm_account),
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
