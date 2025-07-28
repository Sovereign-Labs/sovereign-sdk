use std::convert::Infallible;

use sov_bank::config_gas_token_id;
use sov_mock_da::MockAddress;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, BatchSequencerOutcome, PrivateKey, PublicKey, TxProcessingError};
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::generators::bank::get_default_token_id;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::TestSpec;

use super::da_simulation::{
    simulate_da_with_bad_nonce, simulate_da_with_bad_serialization, simulate_da_with_bad_sig,
    simulate_da_with_revert_msg,
};
use super::optimistic_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{
    default_rewards, has_tx_events, new_test_blob_from_batch, reset_constants,
};

fn assert_outcome(outcome: &BatchSequencerOutcome) {
    assert_eq!(outcome.rewards.accumulated_reward, 0);
    assert!(outcome.rewards.accumulated_penalty > 0);
}

#[test]
fn test_tx_revert() -> Result<(), Infallible> {
    reset_constants();
    // Test checks:
    //  - Batch is successfully applied even with incorrect txs
    //  - Nonce for bad transactions has increased

    let (mut runner, users, sequencer) = setup(1);

    let admin = users.first().unwrap();
    let admin_address = admin.address();
    let admin_key = admin.private_key.clone();
    let sequencer_rollup_address = sequencer.user_info.address();

    let txs = simulate_da_with_revert_msg(admin_key.clone());
    let blob = new_test_blob_from_batch(txs, sequencer.da_address.as_ref());

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let apply_block_result = runner.execute(relevant_blobs);

    assert_eq!(1, apply_block_result.0.batch_receipts.len());
    let apply_blob_outcome = apply_block_result.0.batch_receipts[0].clone();

    assert_eq!(
        sov_modules_api::BatchSequencerOutcome {
            rewards: default_rewards(),
        },
        apply_blob_outcome.inner.outcome,
        "Sequencer execution should have succeeded but failed "
    );

    let txn_receipts = apply_block_result.0.batch_receipts[0].tx_receipts.clone();
    // 3 transactions
    // create 1000 tokens
    // transfer 15 tokens
    // transfer 5000 tokens // this should be reverted
    assert!(txn_receipts[0].receipt.is_successful());
    assert!(txn_receipts[1].receipt.is_successful());
    assert!(txn_receipts[2].receipt.is_reverted());

    // Checks on storage after execution
    runner.query_state(|state| {
        let runtime = IntegTestRuntime::<TestSpec>::default();

        let resp = runtime
            .bank
            .get_balance_of(
                &admin_address,
                get_default_token_id::<TestSpec>(&admin_address),
                state,
            )
            .unwrap();

        assert_eq!(resp, Some(Amount::new(985)));

        let resp = runtime
            .sequencer_registry
            .get_sequencer_address(sequencer.da_address, state)
            .unwrap();
        // Sequencer is not excluded from the list of allowed!
        assert_eq!(Some(sequencer_rollup_address), resp);

        let latest_generation = runtime
            .uniqueness
            .next_generation(&admin_key.pub_key().credential_id(), state)
            .unwrap();

        // with 3 transactions, the latest generation should be 2, because generators send
        // one transaction per generation. So the next generation should be 3
        // The minter account should have its nonce increased for 3 transactions
        assert_eq!(3, latest_generation);
    });

    Ok(())
}

#[test]
fn test_tx_bad_signature() -> Result<(), Infallible> {
    reset_constants();
    let (mut runner, users, sequencer) = setup(1);
    let admin = users.first().unwrap();
    let admin_key = admin.private_key.clone();

    let txs = simulate_da_with_bad_sig(admin_key.clone());

    let blob = new_test_blob_from_batch(txs, sequencer.da_address.as_ref());

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let apply_block_result = runner.execute(relevant_blobs);

    assert_eq!(1, apply_block_result.0.batch_receipts.len());

    let batch_receipt = &apply_block_result.0.batch_receipts[0];

    assert_outcome(&batch_receipt.inner.outcome);

    let tx_receipts = &batch_receipt.tx_receipts;

    assert_eq!(tx_receipts.len(), 1);

    match &tx_receipts[0].receipt {
        sov_modules_api::TxEffect::Skipped(skipped) => {
            assert!(
                matches!(skipped.error, TxProcessingError::AuthenticationFailed(..)),
                "Transaction should fail with an `AuthenticationFailed` error"
            );
        }
        unexpected => panic!("Expected TxEffect::Skipped but got {unexpected:?}"),
    }

    assert_outcome(&batch_receipt.inner.outcome);

    // The batch receipt contains no events.
    assert!(!has_tx_events(batch_receipt));

    runner.query_state(|state| {
        let runtime = &mut IntegTestRuntime::<TestSpec>::default();

        let nonce = runtime
            .uniqueness
            .nonce(&admin_key.pub_key().credential_id(), state)
            .unwrap_infallible()
            .unwrap_or_default();

        assert_eq!(0, nonce);
    });

    Ok(())
}

fn get_attester_stake_for_block(
    sequencer_address: &MockAddress,
    runner: &TestRunner<IntegTestRuntime<TestSpec>, TestSpec>,
) -> u128 {
    runner
        .query_state(|state| {
            let runtime = IntegTestRuntime::<TestSpec>::default();
            runtime
                .sequencer_registry
                .get_sender_balance_via_api(sequencer_address, state)
                .expect("The sequencer should be registered")
        })
        .0
}

/// This test ensures that the sequencer gets penalized for submitting a proof that has a wrong nonce.
#[test]
fn test_tx_bad_nonce() {
    reset_constants();
    let (mut runner, users, sequencer) = setup(1);
    let admin = users.first().unwrap();
    let admin_key = admin.private_key.clone();

    let txs = simulate_da_with_bad_nonce(admin_key);

    let blob = new_test_blob_from_batch(txs, sequencer.da_address.as_ref());

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let initial_sequencer_stake = get_attester_stake_for_block(&sequencer.da_address, &runner);

    let apply_block_result = runner.execute(relevant_blobs);

    // When the nonce is not correct, the transaction receipt does not appear in the block
    assert_eq!(1, apply_block_result.0.batch_receipts.len());
    let tx_receipts = apply_block_result.0.batch_receipts[0].tx_receipts.clone();
    // Bad nonce means that the transaction has to be reverted

    match &tx_receipts[0].receipt {
        sov_modules_api::TxEffect::Successful(_) => (),
        receipt => panic!(
            "Expected first transaction to be Successful error, but got a different TxEffect: {receipt:?}"
        ),
    }

    match &tx_receipts[1].receipt {
        sov_modules_api::TxEffect::Skipped(skipped) => {
            assert!(matches!(
                skipped.error,
                TxProcessingError::CheckUniquenessFailed(..)
            ));
        }
        receipt => panic!("Expected Skipped error, but got a different TxEffect: {receipt:?}"),
    }

    // We don't slash the sequencer for a bad nonce, since the nonce change might have
    // happened while the transaction was in-flight. However, we do *penalize* the sequencer
    // in this case.
    // We're asserting that here to track if the logic changes

    // Since the sequencer is penalized, he is rewarded with 0 tokens.
    let sequencer_outcome = apply_block_result.0.batch_receipts[0].inner.clone().outcome;
    assert_outcome(&sequencer_outcome);
    // We can check that the sequencer staked amount went down.

    let final_sequencer_stake = get_attester_stake_for_block(&sequencer.da_address, &runner);

    assert!(
            final_sequencer_stake < initial_sequencer_stake,
            "The sequencer stake should have decreased, final_sequencer_stake = {final_sequencer_stake:?}, initial_sequencer_stake = {initial_sequencer_stake:?}"
        );
}

#[test]
fn test_tx_bad_serialization() -> Result<(), Infallible> {
    reset_constants();
    let (mut runner, users, sequencer) = setup(1);
    let admin = users.first().unwrap();
    let admin_key = admin.private_key.clone();
    let sequencer_rollup_address = sequencer.user_info.address();

    let runtime = IntegTestRuntime::<TestSpec>::default();

    let sequencer_balance_before = {
        runner.query_state(|state| {
            runtime
                .bank
                .get_balance_of(&sequencer_rollup_address, config_gas_token_id(), state)
                .unwrap_infallible()
                .unwrap()
        })
    };

    let txs = simulate_da_with_bad_serialization(admin_key.clone());
    let blob = new_test_blob_from_batch(txs, sequencer.da_address.as_ref());

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let apply_block_result = runner.execute(relevant_blobs);

    assert_eq!(1, apply_block_result.0.batch_receipts.len());

    let batch_receipt = &apply_block_result.0.batch_receipts[0];

    assert_outcome(&batch_receipt.inner.outcome);

    let tx_receipts = &batch_receipt.tx_receipts;

    assert_eq!(tx_receipts.len(), 1);

    match &tx_receipts[0].receipt {
        sov_modules_api::TxEffect::Skipped(skipped) => assert!(matches!(
            skipped.error,
            TxProcessingError::AuthenticationFailed(..)
        )),
        unexpected => panic!("Expected TxEffect::Skipped but got {unexpected:?}"),
    }

    assert_outcome(&batch_receipt.inner.outcome);

    // The batch receipt contains no events.
    assert!(!has_tx_events(batch_receipt));

    runner.query_state(|state| {
        let sequencer_balance_after = runtime
            .bank
            .get_balance_of(&sequencer_rollup_address, config_gas_token_id(), state)
            .unwrap_infallible()
            .unwrap();
        assert_eq!(sequencer_balance_before, sequencer_balance_after);
    });

    Ok(())
}
