use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::CredentialId;
use sov_test_utils::{TransactionTestCase, TxProcessingError};
use sov_uniqueness::Uniqueness;

use crate::runtime::S;
use crate::utils::{generate_default_tx, setup};

#[test]
fn send_tx_works_window() {
    let (admin, mut runner, evm_account) = setup();
    let admin_credential_id: CredentialId = admin.credential_id();

    runner.query_visible_state(|state| {
        assert_eq!(
            Uniqueness::<S>::default()
                .next_window_nonce(&admin_credential_id, state)
                .unwrap(),
            0,
            "The next window nonce for a new account should start at 0"
        );
    });

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(0), &admin, &evm_account),
        assert: Box::new(move |ctx, state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "First window transaction should succeed"
            );

            assert_eq!(
                Uniqueness::<S>::default()
                    .next_window_nonce(&admin_credential_id, state)
                    .unwrap(),
                0,
                "Window start should still be 0 after marking nonce 0"
            );
        }),
    });

    // Out-of-order nonce should also work
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(5), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Out-of-order window nonce should succeed"
            );
        }),
    });
}

#[test]
fn send_tx_window_duplicate_rejected() {
    let (admin, mut runner, evm_account) = setup();

    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(42), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            assert!(ctx.tx_receipt.is_successful());
        }),
    });

    // Same nonce again should be rejected
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(42), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            let sov_modules_api::TxEffect::Skipped(skipped) = &ctx.tx_receipt else {
                panic!("Expected Skipped, got {:?}", ctx.tx_receipt);
            };
            assert!(
                matches!(skipped.error, TxProcessingError::CheckUniquenessFailed(_)),
                "Should fail uniqueness check"
            );
        }),
    });
}

#[test]
fn send_tx_window_too_far_ahead_rejected() {
    let (admin, mut runner, evm_account) = setup();

    // Nonce 2000 is beyond the 1024-bit window starting at 0
    runner.execute_transaction(TransactionTestCase {
        input: generate_default_tx(UniquenessData::Window(2000), &admin, &evm_account),
        assert: Box::new(move |ctx, _state| {
            let sov_modules_api::TxEffect::Skipped(skipped) = &ctx.tx_receipt else {
                panic!("Expected Skipped, got {:?}", ctx.tx_receipt);
            };
            assert!(
                matches!(skipped.error, TxProcessingError::CheckUniquenessFailed(_)),
                "Nonce too far ahead should fail uniqueness check"
            );
        }),
    });
}
