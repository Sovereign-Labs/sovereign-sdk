use sov_blob_storage::config_deferred_slots_count;
use sov_test_utils::{BatchType, SequencerInfo};

use crate::helpers_soft_confirmations::{
    assert_blobs_are_correctly_received_soft_confirmation, build_soft_confirmation_blobs,
    setup_soft_confirmation_kernel, setup_with_registration_soft_confirmation_kernel, SoftConfRT,
};
use crate::{
    assert_blobs_are_correctly_received_helper, HashMap, SequenceInfo, TestData, TestRunner,
};

/// Test that when the preferred sequencer is slashed, the visible rollup height increases by two until
/// it catches up. For this test to work [`DEFERRED_SLOTS_COUNT`] must be greater than 2.
#[test]
fn test_recovery_mode() {
    let (_, mut runner) = setup_soft_confirmation_kernel();

    // Let's first advance the visible slot to ensure that the sequencer needs to catch up
    // We have to stop before the `DEFERRED_SLOT_COUNT` is reached because otherwise the visible slot
    // number will automatically increase.
    runner.advance_slots((config_deferred_slots_count() - 2) as usize);

    // First let's slash the preferred sequencer by sending a malformed blob.
    // In soft-confirmation mode, sending basic blobs is not allowed and will cause
    // the sequencer to be slashed.
    runner.execute(BatchType(vec![]));

    // Until it catches up, the visible rollup height should increase by two
    let mut expected_visible_slot_increases = vec![2; (config_deferred_slots_count() - 1) as usize];
    // Then the visible rollup height should only increase by one
    expected_visible_slot_increases.push(1);

    // Let's ensure that the visible rollup height increases by two until it catches up
    assert_blobs_are_correctly_received_soft_confirmation(
        // We are not sending any blobs, we just want to assert the way the visible rollup height increases
        vec![],
        // Since we are not sending any blobs, we don't expect any receipts
        vec![vec![]; config_deferred_slots_count() as usize],
        expected_visible_slot_increases,
        &mut runner,
    );
}

/// This test is similar to the previous one, but we have a few batches of deferred blobs to process.
/// We test the following scenario (works if [`DEFERRED_SLOTS_COUNT`] is greater than 2):
/// - Slot 1: Send [(Batch 0, Non-preferred), (Batch 1, Non-preferred)]. Receive []
/// - Slot 2: Send [(Batch 2, Non-preferred)]. Receive []
/// - Slot 3: Slash the preferred sequencer. Receive []
/// - Slot 4: Send []. Receive [Batch 0, Batch 1, Batch 2] (the visible rollup height increases by two)
///
/// Note: we have to manually build the blobs because we don't have a helper method that slashes the sequencer
/// and sends the blobs.
#[test]
fn test_recovery_mode_with_deferred_blobs() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer,
            ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();

    let mut nonces = HashMap::new();

    // Let's first send batches of deferred blobs
    let deferred_slots = [
        vec![
            (regular_sequencer.clone(), SequencerInfo::Regular),
            (regular_sequencer.clone(), SequencerInfo::Regular),
        ],
        vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
    ];

    let mut slots_to_send = deferred_slots
        .iter()
        .map(|blobs_slot_info| build_soft_confirmation_blobs(blobs_slot_info, &mut nonces, 0))
        .collect::<Vec<_>>();

    let (slashing_slot, _) = TestRunner::<SoftConfRT>::batches_to_blobs(
        vec![(BatchType(vec![]), preferred_sequencer.da_address)],
        &mut nonces,
    );

    slots_to_send.push(slashing_slot);

    assert_blobs_are_correctly_received_helper(
        slots_to_send,
        vec![
            vec![],
            vec![],
            vec![],
            vec![
                SequenceInfo::standard(0),
                SequenceInfo::standard(1),
                SequenceInfo::standard(2),
            ],
        ],
        vec![0, 0, 0, 2],
        &mut runner,
    );
}
