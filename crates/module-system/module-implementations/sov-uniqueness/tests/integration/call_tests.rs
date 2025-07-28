use sov_modules_api::macros::config_value;
use sov_modules_api::{CredentialId, TxEffect};
use sov_test_utils::{BatchType, SlotInput, TransactionTestCase, TxProcessingError};
use sov_uniqueness::Uniqueness;

use crate::runtime::S;
use crate::utils::{generate_value_setter_tx, setup};

/// This test verifies that the `MAX_STORED_TX_HASHES_PER_CREDENTIAL` limit is respected and that authentication succeeds when that limit is not exceeded.
#[test]
fn test_max_stored_tx_hashes_per_credential_lite() {
    // This checks that the "MAX_STORED_TX_HASHES_PER_CREDENTIAL" constant is respected
    // and behaves as expected. However, it cannot detect whether your constants.toml configuration
    // makes it possible to exceed `MAX_SEQUENCER_EXEC_GAS_PER_TX` before hitting `MAX_STORED_TX_HASHES_PER_CREDENTIAL`,
    // which would put your account in an unrecoverable state.
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_STORED_TX_HASHES_PER_CREDENTIAL",
        "11",
    );
    do_max_stored_tx_hashes_per_credential_test()
}

#[test]
#[ignore = "This test is long-running, but can detect issues in constants.toml. Run before deploying."]
/// This test checks an account hits `MAX_STORED_TX_HASHES_PER_CREDENTIAL` before maxxing
/// out `MAX_SEQUENCER_EXEC_GAS_PER_TX` for the uniqueness check. This ensures that accounts
/// cannot be put into an unrecoverable state by failing to increment their generation number
/// in a timely manner under the real constants.toml configuration.
///
/// Along the way, this test also checks that the `MAX_STORED_TX_HASHES_PER_CREDENTIAL` limit behaves as expected.
fn test_max_stored_tx_hashes_per_credential() {
    do_max_stored_tx_hashes_per_credential_test()
}

/// This function generates a number of transactions that will fill up the "bucket" of stored transaction hashes
/// for a given account. Then it tries to send one more transaction with a current generation number and verifies that
/// the tx is rejected because the bucket is full. Finally, it updates the generation number and verifies that the
/// next transaction is accepted.
fn do_max_stored_tx_hashes_per_credential_test() {
    let (admin, mut runner, _) = setup();
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
    let max_stored_tx_hashes_per_credential = config_value!("MAX_STORED_TX_HASHES_PER_CREDENTIAL");
    let num_generations = config_value!("PAST_TRANSACTION_GENERATIONS");
    let txs_per_generation = max_stored_tx_hashes_per_credential / num_generations;
    let extra_txs_in_first_generation = max_stored_tx_hashes_per_credential % num_generations;
    let mut txs = vec![];

    // Generate txs to fill up our "bucket" of stored transaction hashes.
    for i in 0..txs_per_generation {
        for generation in 0..num_generations {
            txs.push(generate_value_setter_tx(generation, i as u32, &admin));
        }
    }
    // We divided txs evenly across generations - if there was a remainder, account for it by putting the
    // extra txs in the first bucket.
    for i in 0..extra_txs_in_first_generation {
        txs.push(generate_value_setter_tx(
            0,
            (i + txs_per_generation) as u32,
            &admin,
        ))
    }
    // Execute all the txs in one batch. This is much faster than executing them one by one.
    let batch = SlotInput::Batch(BatchType::from(txs));
    let (slot, _) = runner.execute(batch);
    assert_eq!(
        slot.batch_receipts[0].tx_receipts.len(),
        max_stored_tx_hashes_per_credential as usize
    );
    for (i, tx_receipt) in slot.batch_receipts[0].tx_receipts.iter().enumerate() {
        assert!(
            tx_receipt.receipt.is_successful(),
            "Transaction {i} should be successful but failed"
        );
    }

    // Send one more transaction with a current generation number.
    // This transaction should be skipped because it would cause the bucket to overflow.
    runner.execute_transaction(TransactionTestCase {
        input: generate_value_setter_tx(0, u32::MAX, &admin),
        assert: Box::new(move |ctx, _| {
            let TxEffect::Skipped(skipped) = ctx.tx_receipt else {
                panic!("Transaction should be skipped");
            };
            match skipped.error {
                TxProcessingError::CheckUniquenessFailed(reason) => {
                    assert!(reason.contains("Too many transactions for credential_id"));
                }
                _ => {
                    panic!("Transaction should be rejected because it's not unique");
                }
            }
        }),
    });

    // Increment the generation number. Now the transaction should be accepted because it won't cause the bucket to overflow.
    // Note that we need to add 1 to the number of generations because we have a strict inequality comparison for buckets.
    runner.execute_transaction(TransactionTestCase {
        input: generate_value_setter_tx(num_generations + 1, txs_per_generation as u32, &admin),
        assert: Box::new(move |ctx, _| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Transaction should be successful"
            );
        }),
    });
}
