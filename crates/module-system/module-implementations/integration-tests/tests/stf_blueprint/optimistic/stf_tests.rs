use std::convert::Infallible;
use std::vec;

use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, BatchSequencerOutcome, FullyBakedTx, Spec};
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::generators::bank::get_default_token_id;
use sov_test_utils::TestSpec;

use super::da_simulation::{
    simulate_da, simulate_da_with_incorrect_direct_registration_msg,
    simulate_da_with_multiple_direct_registration_msg,
};
use super::optimistic_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{
    default_rewards, has_tx_events, new_test_blob_for_direct_registration,
    new_test_blob_from_batch, reset_constants, S,
};

#[test]
fn test_demo_values_in_db() -> Result<(), Infallible> {
    reset_constants();
    let (mut runner, users, sequencer) = setup(1);
    let admin = users.first().unwrap();
    let admin_address: <TestSpec as Spec>::Address = admin.address();
    let admin_private_key = admin.private_key.clone();

    let txs = simulate_da(admin_private_key);
    let blob = new_test_blob_from_batch(txs, sequencer.da_address.as_ref());

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let result = runner.execute(relevant_blobs);

    assert_eq!(1, result.0.batch_receipts.len());
    // 2 transactions from value setter
    // 2 transactions from bank
    assert_eq!(4, result.0.batch_receipts[0].tx_receipts.len());

    let apply_blob_outcome = result.0.batch_receipts[0].clone();

    assert_eq!(
        BatchSequencerOutcome {
            rewards: default_rewards()
        },
        apply_blob_outcome.inner.outcome,
        "Sequencer execution should have succeeded but failed "
    );

    assert!(has_tx_events(&apply_blob_outcome),);

    // Generate a new storage instance after dumping data to the db.

    runner.query_state(|state| {
        let runtime = IntegTestRuntime::<TestSpec>::default();
        let resp = runtime
            .bank
            .supply_of(None, get_default_token_id::<S>(&admin_address), state);
        assert_eq!(
            resp.unwrap(),
            sov_bank::TotalSupplyResponse {
                amount: Some(Amount::new(1000))
            }
        );

        assert_eq!(
            runtime.value_setter.value.get(state).unwrap_infallible(),
            Some(33)
        );
    });

    Ok(())
}

#[test]
fn test_demo_values_in_cache() -> Result<(), Infallible> {
    reset_constants();
    let (mut runner, users, sequencer) = setup(1);
    let admin = users.first().unwrap();
    let admin_address: <TestSpec as Spec>::Address = admin.address();
    let admin_private_key = admin.private_key.clone();

    let txs = simulate_da(admin_private_key);

    let blob = new_test_blob_from_batch(txs, sequencer.da_address.as_ref());

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let apply_block_result = runner.execute(relevant_blobs);

    assert_eq!(1, apply_block_result.0.batch_receipts.len());
    let apply_blob_outcome = apply_block_result.0.batch_receipts[0].clone();

    assert_eq!(
        BatchSequencerOutcome {
            rewards: default_rewards()
        },
        apply_blob_outcome.inner.outcome,
        "Sequencer execution should have succeeded but failed"
    );

    assert!(has_tx_events(&apply_blob_outcome),);

    runner.query_state(|state| {
        let runtime = &mut IntegTestRuntime::<TestSpec>::default();

        let resp = runtime
            .bank
            .supply_of(None, get_default_token_id::<S>(&admin_address), state);
        assert_eq!(
            resp.unwrap(),
            sov_bank::TotalSupplyResponse {
                amount: Some(Amount::new(1000))
            }
        );

        assert_eq!(
            runtime.value_setter.value.get(state).unwrap_infallible(),
            Some(33)
        );
    });
    Ok(())
}

// Ensure 1 sequencer be registered per batch
// This test has 2 batches each submitted by unregistered sequencers, given they are in different
// batches then both unregistered sequencers should be registered
#[test]
fn test_multiple_batches_registering_unregistered_sequencers_allows_both_to_register() {
    reset_constants();
    let (mut runner, mut users, _) = setup(1);
    let tx_signer = users.pop().unwrap();

    let direct_sequencer_da_address = MockAddress::new([121; 32]);
    let other_sequencer_da_address = MockAddress::new([86; 32]);

    let mut txs = simulate_da_with_multiple_direct_registration_msg(
        vec![
            direct_sequencer_da_address.as_ref().to_vec(),
            other_sequencer_da_address.as_ref().to_vec(),
        ],
        tx_signer.private_key.clone(),
    );

    let blob1 = MockBlob::new_with_hash(
        borsh::to_vec(&txs.remove(0)).unwrap(),
        MockAddress::new([0; 32]),
    );

    let blob2 = MockBlob::new_with_hash(
        borsh::to_vec(&txs.remove(0)).unwrap(),
        MockAddress::new([0; 32]),
    );

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob1, blob2],
    };

    let apply_block_result = runner.execute(relevant_blobs);

    assert_eq!(2, apply_block_result.0.batch_receipts.len());
    for batch_receipt in apply_block_result.0.batch_receipts.iter() {
        assert_eq!(
            batch_receipt.inner.outcome,
            BatchSequencerOutcome {
                rewards: default_rewards()
            },
        );
        let tx_receipt = &batch_receipt.tx_receipts;

        assert_eq!(1, tx_receipt.len());
        assert!(tx_receipt[0].receipt.is_successful());
    }

    runner.query_state(|state| {
        let runtime = &mut IntegTestRuntime::<TestSpec>::default();

        let successful_reg = runtime
            .sequencer_registry
            .is_registered_sequencer(&direct_sequencer_da_address, state)
            .unwrap();

        assert!(successful_reg);

        let other_seq = runtime
            .sequencer_registry
            .is_registered_sequencer(
                &MockAddress::try_from(other_sequencer_da_address.as_ref()).unwrap(),
                state,
            )
            .unwrap();

        assert!(other_seq);
    });
}

#[test]
fn test_unregistered_sequencer_registration_is_limited_to_one_per_batch() {
    reset_constants();
    let (mut runner, users, _) = setup(1);

    let other_sequencer = users.first().unwrap();

    let other_sequencer_da_address = MockAddress::new([86; 32]);
    let direct_sequencer_da_address = MockAddress::new([121; 32]);

    let txs = simulate_da_with_multiple_direct_registration_msg(
        vec![
            direct_sequencer_da_address.as_ref().to_vec(),
            other_sequencer_da_address.as_ref().to_vec(),
        ],
        other_sequencer.private_key.clone(),
    );

    // ensure there's more than 1 tx. This batch will be rejected,
    assert!(txs.len() > 1);

    // For this test, we need to convert directly from the RawTx to FullyBakedTx so that we can create a batch.
    // We don't have an API for this because the `Batch` struct isn't allowed to contain direct registration transactions.
    let txs = txs
        .into_iter()
        .map(|tx| FullyBakedTx::new(tx.data))
        .collect();
    let blob = new_test_blob_from_batch(txs, direct_sequencer_da_address.as_ref());

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let apply_block_result = runner.execute(relevant_blobs);

    // Ensure that the batch was rejected for containing too many txs.
    assert_eq!(0, apply_block_result.0.batch_receipts.len());

    runner.query_state(|state| {
        let runtime = &mut IntegTestRuntime::<TestSpec>::default();

        let successful_reg = runtime
            .sequencer_registry
            .is_registered_sequencer(&direct_sequencer_da_address, state)
            .unwrap();

        assert!(!successful_reg);

        let other_seq = runtime
            .sequencer_registry
            .is_registered_sequencer(&other_sequencer_da_address, state)
            .unwrap();

        assert!(!other_seq);
    });
}

#[test]
fn test_unregistered_sequencer_registration_incorrect_call_message() {
    reset_constants();
    let (mut runner, mut users, _) = setup(1);

    let other_sequencer = users.pop().unwrap();

    let direct_sequencer_da_address = MockAddress::new([121; 32]);

    let tx =
        simulate_da_with_incorrect_direct_registration_msg(other_sequencer.private_key.clone());
    let blob =
        new_test_blob_for_direct_registration(tx, direct_sequencer_da_address.as_ref(), [0; 32]);
    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let apply_block_result = runner.execute(relevant_blobs);

    assert_eq!(1, apply_block_result.0.batch_receipts.len());
    let receipt = &apply_block_result.0.batch_receipts[0];
    assert_eq!(
        receipt.inner.outcome,
        BatchSequencerOutcome {
            rewards: default_rewards()
        },
    );

    let runtime = &mut IntegTestRuntime::<TestSpec>::default();

    runner.query_state(|state| {
        let registered = runtime
            .sequencer_registry
            .is_registered_sequencer(&direct_sequencer_da_address, state)
            .unwrap();
        assert!(!registered);
    });
}

#[test]
fn test_unregistered_sequencer_batches_are_limited_to_the_configured_amount_per_slot() {
    reset_constants();
    let (mut runner, mut users, _) = setup(1);

    let other_sequencer_da_address = MockAddress::new([86; 32]);
    let direct_sequencer_da_address = MockAddress::new([121; 32]);

    let other_sequencer = users.pop().unwrap();

    let unregistered_blobs_per_slot = 5;
    let mut blobs = vec![];

    let register_tx = simulate_da_with_multiple_direct_registration_msg(
        vec![direct_sequencer_da_address.as_ref().to_vec()],
        other_sequencer.private_key.clone(),
    );

    blobs.push(new_test_blob_for_direct_registration(
        register_tx[0].clone(),
        direct_sequencer_da_address.as_ref(),
        [0; 32],
    ));

    // fill the unregistered blobs per slot quota with invalid messages
    for _ in 0..unregistered_blobs_per_slot {
        let tx =
            simulate_da_with_incorrect_direct_registration_msg(other_sequencer.private_key.clone());
        let blob = new_test_blob_for_direct_registration(
            tx,
            direct_sequencer_da_address.as_ref(),
            [0; 32],
        );
        blobs.push(blob);
    }

    // ensure we have too many blobs
    assert!(blobs.len() > unregistered_blobs_per_slot);

    // this one is outside the limit of allowed unregistered blobs
    // the sequencer should not be registered and this blob should not have been executed
    let register_tx2 = simulate_da_with_multiple_direct_registration_msg(
        vec![other_sequencer_da_address.as_ref().to_vec()],
        other_sequencer.private_key.clone(),
    );

    blobs.push(new_test_blob_for_direct_registration(
        register_tx2[0].clone(),
        direct_sequencer_da_address.as_ref(),
        [0; 32],
    ));

    let relevant_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: blobs,
    };

    let apply_block_result = runner.execute(relevant_blobs);

    assert_eq!(
        unregistered_blobs_per_slot,
        apply_block_result.0.batch_receipts.len()
    );
    // check the first blob, that contained a valid register tx
    let first_registered_receipt = &apply_block_result.0.batch_receipts[0];
    assert_eq!(
        first_registered_receipt.inner.outcome,
        BatchSequencerOutcome {
            rewards: default_rewards()
        },
    );

    // ensure the filler blobs have the right outcome
    for i in 1..unregistered_blobs_per_slot {
        let receipt = &apply_block_result.0.batch_receipts[i];
        assert_eq!(
            receipt.inner.outcome,
            BatchSequencerOutcome {
                rewards: default_rewards()
            },
        );
    }

    // unregistered sequencer tx in the first blob was successfully applied
    runner.query_state(|state| {
        let runtime = &mut IntegTestRuntime::<TestSpec>::default();

        let registered = runtime
            .sequencer_registry
            .is_registered_sequencer(&direct_sequencer_da_address, state)
            .unwrap();

        assert!(registered);

        // unregistered sequencer tx in the blob that fell outside the allowed quota was not applied
        let excessive_blob_sequencer = runtime
            .sequencer_registry
            .is_registered_sequencer(&other_sequencer_da_address, state)
            .unwrap();

        assert!(!excessive_blob_sequencer);
    });
}
