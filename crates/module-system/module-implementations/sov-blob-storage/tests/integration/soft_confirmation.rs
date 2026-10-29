use std::collections::HashMap;
use std::env;

use sov_blob_storage::config_deferred_slots_count;
use sov_mock_da::MockBlob;
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::SequencerInfo;

use crate::helpers_soft_confirmations::{
    assert_blobs_are_correctly_received_soft_confirmation, build_soft_confirmation_blobs,
    setup_soft_confirmation_kernel, setup_with_registration_soft_confirmation_kernel,
};
use crate::{SequenceInfo, TestData};

/// Tests the soft confirmation kernel functionality by executing one batch per slot for the preferred sequencer.
/// Expected result:
/// - Slot 1: Send []. Receive []
/// - Slot 2: Send [Batch 0]. Receive [Batch 0]
/// - Slot 3: Send [Batch 1]. Receive [Batch 1]
/// - Slot 4: Send [Batch 2]. Receive [Batch 2]
#[test]
fn store_and_retrieve_standard_soft_confirmation_kernel() {
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();

    runner.advance_slots(1);

    // Create three slots, each containing a batch of blobs.
    // We should receive three receipts in the same order as the blobs were sent.
    let slots = vec![
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 1,
            },
        )],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 2,
            },
        )],
    ];

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        vec![
            vec![SequenceInfo {
                id: 0,
                sequence_number: Some(0),
            }],
            vec![SequenceInfo {
                id: 1,
                sequence_number: Some(1),
            }],
            vec![SequenceInfo {
                id: 2,
                sequence_number: Some(2),
            }],
        ],
        vec![1, 1, 1],
        &mut runner,
    );
}

/// Create three slots using the preferred sequencer.
/// The first slot contains three batches, the next two slots contain one batch each.
/// We should receive the receipts in the following order:
/// Slot 1: Send [Batch 0, Batch 1, Batch 2]. Receive [Batch 0]
/// Slot 2: Send [Batch 3]. Receive [Batch 1]
/// Slot 3: Send [Batch 4]. Receive [Batch 2]
/// Slot 4: Send []. Receive [Batch 3]
/// Slot 5: Send []. Receive [Batch 4]
#[test]
fn store_and_retrieve_standard_soft_confirmation_kernel_deferred() {
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();

    let slots = vec![
        vec![
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 0,
                },
            ),
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 1,
                },
            ),
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 2,
                },
            ),
        ],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 3,
            },
        )],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 4,
            },
        )],
    ];

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        vec![
            vec![SequenceInfo {
                id: 0,
                sequence_number: Some(0),
            }],
            vec![SequenceInfo {
                id: 1,
                sequence_number: Some(1),
            }],
            vec![SequenceInfo {
                id: 2,
                sequence_number: Some(2),
            }],
            vec![SequenceInfo {
                id: 3,
                sequence_number: Some(3),
            }],
            vec![SequenceInfo {
                id: 4,
                sequence_number: Some(4),
            }],
        ],
        vec![1, 1, 1, 1, 1],
        &mut runner,
    );
}

/// Tests that the blobs from the non-preferred sequencer are deferred.
/// Send a batch from the non-preferred sequencer. We should have the following receipts:
/// Slot 1: Send [Batch 0]. Receive []
/// Slots [1..`DEFERRED_SLOTS_COUNT`-1]: Send []. Receive []
/// Slot [`DEFERRED_SLOTS_COUNT`]: Send []. Receive [Batch 0]
#[test]
fn non_preferred_sequencer_deferred() {
    let (
        TestData {
            regular_sequencer, ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();

    let slots = vec![vec![(regular_sequencer.clone(), SequencerInfo::Regular)]];

    let mut receive_order = vec![vec![]; (config_deferred_slots_count() - 1) as usize];
    receive_order.push(vec![SequenceInfo::standard(0)]);

    let mut visible_slot_heights = vec![0; (config_deferred_slots_count() - 1) as usize];

    visible_slot_heights.push(1);

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        receive_order,
        visible_slot_heights,
        &mut runner,
    );
}

/// Interspace slots between the preferred and the non preferred sequencer. We advance the slots every time with preferred sequencer transactions.
/// Assuming we have a [`DEFERRED_SLOTS_COUNT`] == 5,
/// We simulate the following scenario:
/// - Slot 1: Send [(Pref, Blob 0), (Standard, Blob 1), (Pref, Blob 2) | Recv [Blob 0, Blob 1]
/// - Slot 2: Send [(Pref, Blob 3)], (Standard, Blob 4), (Standard, Blob 5)] | Recv [Blob 2, Blob 4, Blob 5]
/// - Slot 3: Send [(Pref, Blob 6)] | Recv [Blob 3]
/// - Slot 5: Send [] | Recv [Blob 6]
#[test]
fn interspace_slots_preferred_non_preferred_sequencer_increase_slots() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer,
            ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();

    let slots = vec![
        vec![
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 1,
                },
            ),
            (regular_sequencer.clone(), SequencerInfo::Regular),
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 2,
                },
            ),
        ],
        vec![
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 3,
                },
            ),
            (regular_sequencer.clone(), SequencerInfo::Regular),
            (regular_sequencer.clone(), SequencerInfo::Regular),
        ],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 4,
            },
        )],
    ];

    let receive_order = vec![
        vec![
            SequenceInfo {
                id: 0,
                sequence_number: Some(1),
            },
            SequenceInfo::standard(1),
        ],
        vec![
            SequenceInfo {
                id: 2,
                sequence_number: Some(2),
            },
            SequenceInfo::standard(4),
            SequenceInfo::standard(5),
        ],
        vec![SequenceInfo {
            id: 3,
            sequence_number: Some(3),
        }],
        vec![SequenceInfo {
            id: 6,
            sequence_number: Some(4),
        }],
    ];

    let visible_slot_heights_increases = vec![1, 1, 1, 1];

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        receive_order,
        visible_slot_heights_increases,
        &mut runner,
    );
}

/// Interspace slots between the preferred and the non preferred sequencer. We don't advance the slots every time with preferred sequencer transactions.
/// Assuming we have a [`DEFERRED_SLOTS_COUNT`] == 5,
/// We simulate the following scenario:
/// - Slot 1: Send [(Pref, Blob 0), (Standard, Blob 1), (Pref, Blob 2) | Recv [Blob 0, Blob 1]
/// - Slot 2: Send [(Standard, Blob 3)], (Standard, Blob 4), (Standard, Blob 5)] | Recv [Blob 2, Blob 3, Blob 4, Blob 5]
/// - Slot 3: Send [(Standard, Blob 6)] | Recv []
/// - Slot 4: Send [] | Recv []
/// - Slot 5: Send [] | Recv []
/// - Slot 6: Send [] | Recv []
/// - Slot 7: Send [] | Recv [Blob 6]
#[test]
fn interspace_slots_preferred_non_preferred_sequencer_dont_advance_slots() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer,
            ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();

    let slots = vec![
        vec![
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 1,
                },
            ),
            (regular_sequencer.clone(), SequencerInfo::Regular),
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 2,
                },
            ),
        ],
        vec![
            (regular_sequencer.clone(), SequencerInfo::Regular),
            (regular_sequencer.clone(), SequencerInfo::Regular),
            (regular_sequencer.clone(), SequencerInfo::Regular),
        ],
        vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
    ];

    let mut receive_order = vec![
        vec![
            SequenceInfo {
                id: 0,
                sequence_number: Some(1),
            },
            SequenceInfo::standard(1),
        ],
        vec![
            SequenceInfo {
                id: 2,
                sequence_number: Some(2),
            },
            SequenceInfo::standard(3),
            SequenceInfo::standard(4),
            SequenceInfo::standard(5),
        ],
    ];
    receive_order.append(&mut vec![
        vec![];
        (config_deferred_slots_count() - 1) as usize
    ]);
    receive_order.push(vec![SequenceInfo::standard(6)]);

    let mut visible_slot_heights_increases = vec![1, 1];
    visible_slot_heights_increases
        .append(&mut vec![0; (config_deferred_slots_count() - 1) as usize]);
    visible_slot_heights_increases.push(1);

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        receive_order,
        visible_slot_heights_increases,
        &mut runner,
    );
}

/// Test that the preferred sequencer is able to force execute blobs
/// We test the following scenario:
/// - Slot 1: Send [(Batch 0, Regular)]. Receive []
/// - Slot 2: Send [(Batch 1, Regular)]. Receive []
/// - Slot 3: Send [(Batch 2, Regular)]. Receive []
/// - Slot 4: Send [(Batch 3, Preferred, Height increase 2). Receive [Batch 3, Batch 0, Batch 1]]
///
/// This test assumes that [`DEFERRED_SLOTS_COUNT`] is greater than 2.
#[test]
fn send_slots_with_high_deferred_slot_adjustment() {
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer,
            ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();

    let slots_info = vec![
        vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
        vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
        vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 2,
                sequence_number: 1,
            },
        )],
    ];

    let receive_order = vec![
        vec![],
        vec![],
        vec![],
        vec![
            SequenceInfo {
                id: 3,
                sequence_number: Some(1),
            },
            SequenceInfo::standard(0),
            SequenceInfo::standard(1),
        ],
    ];

    let visible_slot_heights_increases = vec![0, 0, 0, 2];

    assert_blobs_are_correctly_received_soft_confirmation(
        slots_info,
        receive_order,
        visible_slot_heights_increases,
        &mut runner,
    );
}

/// When sending a blob with an outdated sequencer number, the blob should be dropped.
#[test]
fn blobs_with_low_sequencer_number_get_dropped() {
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();

    let slots = vec![
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
        // This blob should be dropped because, after the blob above, the sequence number becomes 2.
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
    ];

    let receive_order = vec![
        vec![SequenceInfo {
            id: 0,
            sequence_number: Some(0),
        }],
        vec![],
    ];

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        receive_order,
        vec![1, 0],
        &mut runner,
    );
}

/// Check that blobs with higher sequencer numbers get deferred.
/// We test the following situation:
/// - Slot 1: Send [(Batch 0, Priority, Sequencer number 2)]. Receive []
/// - Slot 2: Send [(Batch 1, Priority, Sequencer number 1)]. Receive []
/// - Slot 3: Send [(Batch 2, Priority, Sequencer number 0)]. Receive [Batch 2]
/// - Slot 4: Send []. Receive [Batch 1]
/// - Slot 5: Send []. Receive [Batch 0]
#[test]
fn blobs_with_high_sequencer_number_get_deferred() {
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();

    let slots = vec![
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 2,
            },
        )],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 1,
            },
        )],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
    ];

    let receive_order = vec![
        vec![],
        vec![],
        vec![SequenceInfo {
            id: 2,
            sequence_number: Some(0),
        }],
        vec![SequenceInfo {
            id: 1,
            sequence_number: Some(1),
        }],
        vec![SequenceInfo {
            id: 0,
            sequence_number: Some(2),
        }],
    ];

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        receive_order,
        vec![0, 0, 1, 1, 1],
        &mut runner,
    );
}

/// Check that blobs with higher sequencer numbers get deferred.
/// We test the following situation:
/// - Slot 1: Send [(Batch 0, Priority, Sequencer number 2), (Batch 1, Priority, Sequencer number 1)]. Receive []
/// - Slot 2: Send []. Receive []
/// - Slot 3: Send [(Batch 2, Priority, Sequencer number 0)]. Receive [Batch 2]
/// - Slot 4: Send []. Receive [Batch 1]
/// - Slot 5: Send []. Receive [Batch 0]
#[test]
fn blobs_with_out_of_order_sequencer_number_in_single_slot_get_deferred() {
    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();

    let slots = vec![
        vec![
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 2,
                },
            ),
            (
                preferred_sequencer.clone(),
                SequencerInfo::Preferred {
                    slots_to_advance: 1,
                    sequence_number: 1,
                },
            ),
        ],
        vec![],
        vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
    ];

    let receive_order = vec![
        vec![],
        vec![],
        vec![SequenceInfo {
            id: 2,
            sequence_number: Some(0),
        }],
        vec![SequenceInfo {
            id: 1,
            sequence_number: Some(1),
        }],
        vec![SequenceInfo {
            id: 0,
            sequence_number: Some(2),
        }],
    ];

    assert_blobs_are_correctly_received_soft_confirmation(
        slots,
        receive_order,
        vec![0, 0, 1, 1, 1],
        &mut runner,
    );
}

#[test]
fn check_blob_selection() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "1000",
    );

    let (
        TestData {
            preferred_sequencer,
            ..
        },
        mut runner,
    ) = setup_soft_confirmation_kernel();

    let mut nonces = HashMap::new();

    let slot_to_send = build_soft_confirmation_blobs(
        &vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
        &mut nonces,
        700,
    );

    let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);
    assert_eq!(result.0.batch_receipts.len(), 1);

    let slot_to_send = build_soft_confirmation_blobs(
        &vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 0,
            },
        )],
        &mut nonces,
        1001,
    );

    let result = runner.execute::<RelevantBlobs<MockBlob>>(slot_to_send);
    assert_eq!(result.0.batch_receipts.len(), 0);
}

/// The user state root must change only when a rollup block is created. Deferring a blob from a
/// registered non-preferred sequencer escrows part of its stake, and that escrow must be tracked in
/// kernel state only. Otherwise the node's user root changes in a slot with no rollup block, and
/// the preferred sequencer's user root consistency check against the node fails.
#[test]
fn deferring_registered_non_preferred_blob_does_not_change_user_state() {
    use sov_sequencer_registry::SequencerRegistry;
    use sov_state::{ProvableNamespace, StateRoot};

    let (
        TestData {
            regular_sequencer, ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();

    let sequencer_stake = |runner: &crate::TestRunner<_>| {
        runner.query_state(|state| {
            SequencerRegistry::<crate::S>::default()
                .get_sender_balance_via_api(&regular_sequencer.da_address, state)
                .expect("The regular sequencer must be registered")
        })
    };
    let stake_before = sequencer_stake(&runner);
    let user_root_before = runner.state_root().namespace_root(ProvableNamespace::User);

    // No preferred batch is in this slot, so the blob is deferred and no rollup block is created.
    let blobs = build_soft_confirmation_blobs(
        &vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
        &mut HashMap::new(),
        0,
    );
    runner.execute::<RelevantBlobs<MockBlob>>(blobs);

    assert!(
        runner.receipts().last().unwrap().batch_receipts.is_empty(),
        "The non-preferred blob should be deferred, not executed"
    );
    assert!(
        sequencer_stake(&runner) < stake_before,
        "Deferring the blob should escrow part of the sequencer's stake"
    );
    assert_eq!(
        runner.state_root().namespace_root(ProvableNamespace::User),
        user_root_before,
        "The user state root must not change in a slot that creates no rollup block"
    );
}

/// Regression test: a deferred blob from a *registered* non-preferred sequencer is stored with its
/// gas in a deferred escrow. When the preferred sequencer's block builder pulls it via
/// `get_non_preferred_blobs`, the escrow must be released exactly as the node does it (instead of
/// panicking in `into_selected_blob`), leaving the gas for the blob's pre-execution checks
/// reserved.
#[test]
fn sequencer_get_non_preferred_blobs_releases_deferred_escrow() {
    use sov_modules_api::capabilities::{BlobSelector, ChainState, HasKernel};
    use sov_modules_api::{Gas, GasArray, GasSpec, KernelStateAccessor, NoOpControlFlow};
    use sov_rollup_interface::common::SlotNumber;

    use crate::helpers_soft_confirmations::SoftConfRT;

    let (
        TestData {
            regular_sequencer, ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();

    // The registered non-preferred sequencer posts a single-tx batch. No preferred batch is in
    // this slot, so the blob is deferred and its gas is held in a deferred escrow.
    let blobs = build_soft_confirmation_blobs(
        &vec![(regular_sequencer.clone(), SequencerInfo::Regular)],
        &mut HashMap::new(),
        0,
    );
    runner.execute::<RelevantBlobs<MockBlob>>(blobs);
    assert!(
        runner.receipts().last().unwrap().batch_receipts.is_empty(),
        "The non-preferred blob should be deferred, not executed"
    );

    // Do what the preferred sequencer's block builder does when its visible slot advances over
    // the DA slot that contains the deferred blob.
    runner.__apply_to_state(|checkpoint| {
        let mut rt = SoftConfRT::default();
        let mut kernel = rt.kernel();
        let mut accessor = KernelStateAccessor::from_checkpoint(&kernel, checkpoint);
        let next_visible_slot_number = accessor.visible_slot_number().advance(10);
        kernel.increment_rollup_height(&mut accessor, next_visible_slot_number);
        let gas_price = kernel.base_fee_per_gas(&mut accessor).unwrap();
        let blobs = kernel.get_non_preferred_blobs(
            SlotNumber::new(1).range_inclusive(SlotNumber::new(10)),
            &mut accessor,
            NoOpControlFlow,
        );

        assert_eq!(blobs.len(), 1, "The deferred blob should be selected");
        // As on the node, the escrow pays for one pre-execution check per tx of the batch.
        let one_check = <crate::S as GasSpec>::max_tx_check_costs()
            .checked_scalar_product(1)
            .unwrap()
            .checked_value(gas_price)
            .unwrap();
        assert_eq!(
            blobs[0].reserved_gas_tokens,
            Some(one_check),
            "The blob should reserve the gas for its pre-execution checks, moved out of the deferred escrow"
        );
    });
}
