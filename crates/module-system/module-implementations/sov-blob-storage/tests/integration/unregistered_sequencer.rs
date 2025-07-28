use std::num::NonZero;

use sov_blob_storage::{config_deferred_slots_count, config_unregistered_blobs_per_slot};
use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::{Amount, CryptoSpec, Spec};
use sov_modules_stf_blueprint::{BatchReceipt, Runtime};
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::SequencerRegistry;
use sov_test_utils::runtime::traits::MinimalGenesis;
use sov_test_utils::{
    default_test_tx_details, AsUser, EncodeCall, SequencerInfo, TestSequencer, TransactionType,
};
use sov_value_setter::ValueSetter;

use crate::helpers_basic_kernel::{build_basic_blobs, setup_basic_kernel, BasicRT};
use crate::helpers_soft_confirmations::{
    build_soft_confirmation_blobs, setup_soft_confirmation_kernel,
};
use crate::{HashMap, TestData, S};

fn make_unregistered_blobs<
    RT: Runtime<S> + MinimalGenesis<S> + EncodeCall<SequencerRegistry<S>>,
>(
    num_blobs: u64,
    sender: &TestSequencer<S>,
    nonces: &mut HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64>,
) -> Vec<MockBlob> {
    (0..num_blobs)
        .map(|_| {
            let msg: sov_sequencer_registry::CallMessage<S> =
                sov_sequencer_registry::CallMessage::Register {
                    da_address: sender.da_address,
                    amount: Amount::new(22),
                };

            let key = sender.as_user().private_key().clone();
            let details = default_test_tx_details::<S>();

            let tx = TransactionType::<RT, S>::sign_and_serialize(
                <RT as EncodeCall<SequencerRegistry<S>>>::to_decodable(msg),
                key,
                &RT::CHAIN_HASH,
                details,
                nonces,
            );

            MockBlob::new_with_hash(borsh::to_vec(&tx).unwrap(), sender.da_address)
        })
        .collect::<Vec<_>>()
}

fn make_unregistered_blob_with_approx_size<
    RT: Runtime<S> + MinimalGenesis<S> + EncodeCall<ValueSetter<S>>,
>(
    sender: &TestSequencer<S>,
    size: usize,
) -> MockBlob {
    let blob = vec![1; size];
    let msg = sov_value_setter::CallMessage::SetManyValues(blob);
    let key = sender.as_user().private_key().clone();
    let details = default_test_tx_details::<S>();
    let nonces = &mut HashMap::new();

    let tx = TransactionType::<RT, S>::sign_and_serialize(
        <RT as EncodeCall<ValueSetter<S>>>::to_decodable(msg),
        key,
        &RT::CHAIN_HASH,
        details,
        nonces,
    );

    MockBlob::new_with_hash(borsh::to_vec(&tx).unwrap(), sender.da_address)
}

/// Tries to send too many blobs from a non-registered sequencer and hit rate limits.
#[test]
fn blobs_from_non_registered_sequencers_are_limited_to_set_amount() {
    let (
        TestData {
            regular_sequencer: non_registered_sequencer,
            ..
        },
        mut runner,
    ) = setup_basic_kernel();

    let mut nonces = HashMap::new();

    // Make more unregistered blobs than the limit
    let unregistered_blobs = make_unregistered_blobs::<BasicRT>(
        config_unregistered_blobs_per_slot() + 5,
        &non_registered_sequencer,
        &mut nonces,
    );

    let unregistered_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: unregistered_blobs,
    };

    // Send them
    let result = runner.execute::<RelevantBlobs<MockBlob>>(unregistered_blobs);

    // Assert that the number of blobs received is at most the [`UNREGISTERED_BLOBS_PER_SLOT`] limit
    assert_eq!(
        result.0.batch_receipts.len(),
        config_unregistered_blobs_per_slot() as usize,
        "The number of blobs received should be equal to `UNREGISTERED_BLOBS_PER_SLOT`"
    );
}

/// Tries to send too blobs that are too large from a non-registered sequencer.
#[test]
fn blobs_from_non_registered_sequencers_are_limited_in_length() {
    let (
        TestData {
            regular_sequencer: non_registered_sequencer,
            ..
        },
        mut runner,
    ) = setup_basic_kernel();

    // Make and submit an unregistered blob that is too large
    let blob = make_unregistered_blob_with_approx_size::<BasicRT>(&non_registered_sequencer, 1010);
    let unregistered_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let result = runner.execute::<RelevantBlobs<MockBlob>>(unregistered_blobs);
    // Assert that the number of blobs received is 0
    assert_eq!(
        result.0.batch_receipts.len(),
        0,
        "No blobs should be received, since the submitted blob is too large"
    );
}

/// Tries to send too blobs that are too large from a non-registered sequencer.
#[test]
fn blobs_from_non_registered_sequencers_are_not_too_limited_in_length() {
    let (
        TestData {
            regular_sequencer: non_registered_sequencer,
            ..
        },
        mut runner,
    ) = setup_basic_kernel();

    // Make and submit an unregistered blob that is less than 1k and contains a correctly serialized RawTx
    let blob = make_unregistered_blob_with_approx_size::<BasicRT>(&non_registered_sequencer, 500);
    let unregistered_blob = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };
    let result = runner.execute::<RelevantBlobs<MockBlob>>(unregistered_blob);

    // Assert that the number of blobs received is 1
    assert_eq!(
        result.0.batch_receipts.len(),
        1,
        "The number of blobs received should be 1, since the submitted blob is less than 1k and contains a correctly serialized RawTx"
    );
}

#[test]
fn blobs_from_non_registered_sequencers_are_limited_to_set_amount_soft_confirmation() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer: non_registered_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();

    let mut nonces = HashMap::new();

    // Make more unregistered blobs than the limit
    let mut unregistered_blobs = make_unregistered_blobs::<BasicRT>(
        config_unregistered_blobs_per_slot() + 1,
        &non_registered_sequencer,
        &mut nonces,
    );

    let mut slot_to_send = build_soft_confirmation_blobs(
        &vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
        &mut nonces,
        0,
    );

    slot_to_send.batch_blobs.append(&mut unregistered_blobs);

    // Send them
    let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);

    // Assert that the number of blobs received is below the [`UNREGISTERED_BLOBS_PER_SLOT`] limit
    assert_eq!(
        result.0.batch_receipts.len(),
        1 + config_unregistered_blobs_per_slot() as usize,
        "The number of blobs received should be equal to `UNREGISTERED_BLOBS_PER_SLOT` plus 1 (the preferred blob)"
    );
}

#[test]
fn blobs_from_non_registered_sequencers_base_sequencing() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer: non_registered_sequencer,
            ..
        },
        mut runner,
    ) = setup_basic_kernel();

    let mut nonces = HashMap::new();

    // Make more unregistered blobs than the limit
    let unregistered_blobs = make_unregistered_blobs::<BasicRT>(
        config_unregistered_blobs_per_slot() + 1,
        &non_registered_sequencer,
        &mut nonces,
    );

    let mut slot_to_send =
        build_basic_blobs(&vec![(preferred_sequencer.clone(), 0); 4], &mut nonces);

    slot_to_send.batch_blobs.extend(unregistered_blobs);

    // Send them
    let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);

    // Assert that the number of blobs received is below the [`UNREGISTERED_BLOBS_PER_SLOT`] limit
    assert_eq!(
        result.0.batch_receipts.len(),
        4 + config_unregistered_blobs_per_slot() as usize,
        "The number of blobs received should be equal to `UNREGISTERED_BLOBS_PER_SLOT` plus 4 (the registered blobs)"
    );
}

/// Tests that a forced registration from a non-registered sequencer, appearing in the same slot
/// as a preferred empty blob, is correctly deferred and then executed. Specifically:
/// - Creates a single forced registration blob alongside a preferred empty blob in the same slot.
/// - Processes subsequent slots (up to `config_deferred_slots_count() + 5`) to confirm the registration
///   isn’t lost or ignored.
/// - Verifies that all batch receipts remain valid throughout.
#[test]
fn forced_registration_first_on_same_slot_as_preferred_blob() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer: non_registered_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();
    let till_non_preferred_applied = config_deferred_slots_count() + 5;

    // [forced_registration + slot advancing slot] + n of slot advancing bacthed
    let expected_number_of_batches = till_non_preferred_applied + 2;
    let mut nonces = HashMap::new();

    let mut unerigestered_blobs =
        make_unregistered_blobs::<BasicRT>(1, &non_registered_sequencer, &mut nonces);
    let preferred_empty_blobs = vec![build_preferred_empty_blob(
        0,
        1,
        preferred_sequencer.da_address,
    )];
    unerigestered_blobs.extend(preferred_empty_blobs);
    let blobs_to_send = unerigestered_blobs;
    let relevant_blobs = RelevantBlobs::<MockBlob> {
        proof_blobs: Vec::new(),
        batch_blobs: blobs_to_send,
    };
    let mut received_batches = 0;

    let result = runner.execute::<RelevantBlobs<MockBlob>>(relevant_blobs);

    for batch_receipt in result.0.batch_receipts {
        assert_batch_is_not_at_loss(&batch_receipt);
        received_batches += 1;
    }

    for sequence in 1..=till_non_preferred_applied {
        let slot_to_send = RelevantBlobs::<MockBlob> {
            proof_blobs: Vec::new(),
            batch_blobs: vec![build_preferred_empty_blob(
                sequence,
                1,
                preferred_sequencer.da_address,
            )],
        };
        let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);
        for batch_receipt in result.0.batch_receipts {
            assert_batch_is_not_at_loss(&batch_receipt);
            received_batches += 1;
        }
    }

    assert_eq!(received_batches, expected_number_of_batches);
}

fn assert_batch_is_not_at_loss(batch_receipt: &BatchReceipt<S>) {
    assert!(
        batch_receipt.inner.outcome.rewards.accumulated_reward
            >= batch_receipt.inner.outcome.rewards.accumulated_penalty,
        "Loss for batch: {:?}",
        batch_receipt.inner
    );
}

fn build_preferred_empty_blob(
    sequence_number: u64,
    slots_to_advance: u8,
    sequencer_address: MockAddress,
) -> MockBlob {
    let batch_data = sov_blob_storage::PreferredBatchData {
        sequence_number,
        data: vec![].into(),
        visible_slots_to_advance: NonZero::new(slots_to_advance).unwrap(),
    };

    let serialized_blob = borsh::to_vec(&batch_data).unwrap();
    MockBlob::new_with_hash(serialized_blob, sequencer_address)
}
