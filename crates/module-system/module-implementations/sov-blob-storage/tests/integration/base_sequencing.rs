use std::collections::HashMap;
use std::env;

use sov_mock_da::MockBlob;
use sov_rollup_interface::da::RelevantBlobs;

use crate::helpers_basic_kernel::{
    assert_blobs_are_correctly_received_basic_kernel, build_basic_blobs, setup_basic_kernel,
};
use crate::TestData;

/// Tests that the blob storage module can store and retrieve blobs.
/// The test creates a batch of blobs, and then checks that the blobs are stored and retrieved correctly by
/// comparing the hashes of the blobs sent and the hashes of the receipts received.
#[test]
fn store_and_retrieve_standard_basic_kernel() {
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_basic_kernel();

    runner.advance_slots(1);

    // Create three slots, each containing a batch of blobs.
    // We should receive three receipts in the same order as the blobs were sent.
    let slots = vec![
        vec![(preferred_sequencer.clone(), 0); 3],
        vec![(preferred_sequencer.clone(), 0)],
        vec![(preferred_sequencer.clone(), 0)],
    ];

    assert_blobs_are_correctly_received_basic_kernel(
        slots,
        vec![vec![0, 1, 2], vec![3], vec![4]],
        vec![1, 1, 1],
        &mut runner,
    );
}

#[test]
fn check_blob_selection2() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "50000",
    );
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_basic_kernel();

    let mut nonces = HashMap::new();

    {
        let slot_to_send = build_basic_blobs(
            &vec![
                (preferred_sequencer.clone(), 20),
                (preferred_sequencer.clone(), 45),
                (preferred_sequencer.clone(), 10),
                (preferred_sequencer.clone(), 50001),
            ],
            &mut nonces,
        );

        let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);
        assert_eq!(result.0.batch_receipts.len(), 3);
    }

    {
        // First slot bigger than MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE
        let slot_to_send = build_basic_blobs(
            &vec![
                (preferred_sequencer.clone(), 50001),
                (preferred_sequencer.clone(), 50),
            ],
            &mut nonces,
        );

        let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);
        assert_eq!(result.0.batch_receipts.len(), 1);
    }

    // Test the edge cases.
    {
        let slot_to_send = build_basic_blobs(
            &vec![
                (preferred_sequencer.clone(), 5000),
                (preferred_sequencer.clone(), 50),
            ],
            &mut nonces,
        );

        let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);
        assert_eq!(result.0.batch_receipts.len(), 1);
    }

    {
        let slot_to_send = build_basic_blobs(&vec![], &mut nonces);

        let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);
        assert_eq!(result.0.batch_receipts.len(), 0);
    }
}
