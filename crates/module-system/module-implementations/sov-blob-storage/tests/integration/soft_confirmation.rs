use std::collections::HashMap;
use std::env;

use sov_bank::derived_holder::DerivedHolder;
use sov_bank::IntoPayable;
use sov_blob_storage::config_deferred_slots_count;
use sov_mock_da::MockBlob;
use sov_modules_api::{
    Amount, Gas, GasArray, GasSpec, KernelStateAccessor, NoOpControlFlow, RawTx, Spec,
};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::runtime::BlobStorage;
use sov_test_utils::{
    BatchType, SequencerInfo, SoftConfirmationBlobInfo, TestSequencer, TransactionType,
};

use crate::helpers_soft_confirmations::{
    assert_blobs_are_correctly_received_soft_confirmation, build_soft_confirmation_blobs,
    setup_soft_confirmation_kernel, setup_with_registration_soft_confirmation_kernel, SoftConfRT,
};
use crate::{SequenceInfo, TestData, S};

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

/// Escrows of all blobs still in deferred storage. The escrowed tokens are held in the bank
/// balance of each blob's derived holder until the blob leaves storage.
fn deferred_escrows(runner: &crate::TestRunner<SoftConfRT>) -> Vec<(DerivedHolder, Amount)> {
    let true_slot = runner.true_slot_number().get();
    runner.query_state(|state| {
        let mut escrows = Vec::new();
        for slot in 0..=true_slot {
            for holder in BlobStorage::<S>::default()
                .get_deferred_escrow_holders(SlotNumber::new(slot), state)
                .unwrap()
            {
                let amount = sov_bank::Bank::<S>::default()
                    .get_balance_of(holder.to_payable(), sov_bank::config_gas_token_id(), state)
                    .unwrap()
                    .unwrap_or_else(|| {
                        panic!("Deferred blob with holder {holder} has no escrowed balance")
                    });
                escrows.push((holder, amount));
            }
        }
        escrows
    })
}

/// Defers a two-transaction batch from `sequencer`, a registered non-preferred sequencer, and
/// returns the amount escrowed for it. No preferred batch is in the slot, so no rollup block is
/// created. Two transactions, because the escrow pays for one pre-execution check per transaction.
fn defer_two_tx_batch(
    sequencer: &TestSequencer<S>,
    runner: &mut crate::TestRunner<SoftConfRT>,
) -> Amount {
    let blobs = crate::TestRunner::<SoftConfRT>::soft_confirmation_batches_to_blobs(
        vec![SoftConfirmationBlobInfo {
            batch_type: BatchType(vec![TransactionType::PreSigned(RawTx::new(vec![])); 2]),
            sequencer_address: sequencer.da_address,
            sequencer_info: SequencerInfo::Regular,
        }],
        &mut HashMap::new(),
    );
    runner.execute::<RelevantBlobs<MockBlob>>(blobs);

    let escrows = deferred_escrows(runner);
    assert_eq!(
        escrows.len(),
        1,
        "The non-preferred blob should be deferred with an escrow entry"
    );
    escrows[0].1
}

/// Does what the preferred sequencer's block builder does when it starts a rollup block that
/// advances the visible slot by one: increments the rollup height, reads the new base fee and
/// pulls the deferred blobs of the newly visible slot. Returns that base fee and the gas reserved
/// for each pulled blob.
fn replay_deferred_blobs_as_preferred_sequencer(
    runner: &mut crate::TestRunner<SoftConfRT>,
) -> (<<S as Spec>::Gas as Gas>::Price, Vec<Option<Amount>>) {
    use sov_modules_api::capabilities::{BlobSelector as _, ChainState as _, HasKernel as _};

    let mut output = None;
    runner.__apply_to_state(|checkpoint| {
        let mut rt = SoftConfRT::default();
        let mut kernel = rt.kernel();
        let mut accessor = KernelStateAccessor::from_checkpoint(&kernel, checkpoint);
        // `__apply_to_state` has already advanced the visible slot of the checkpoint by one.
        let next_visible_slot_number = accessor.visible_slot_number();
        kernel.increment_rollup_height(&mut accessor, next_visible_slot_number);
        let gas_price = kernel
            .base_fee_per_gas(&mut accessor)
            .expect("The base fee of the rollup block being built must be known");
        let blobs = kernel.get_non_preferred_blobs(
            std::iter::once(next_visible_slot_number.as_true()),
            &gas_price,
            &mut accessor,
            NoOpControlFlow,
        );
        output = Some((
            gas_price,
            blobs
                .into_iter()
                .map(|blob| blob.reserved_gas_tokens)
                .collect(),
        ));
    });
    output.expect("`__apply_to_state` must run the closure")
}

/// Regression test: a deferred blob from a registered non-preferred sequencer is stored with its
/// gas in a deferred escrow. When the preferred sequencer's block builder pulls it via
/// `get_non_preferred_blobs`, the escrow must be released (instead of panicking in
/// `into_selected_blob`), leaving the gas for one pre-execution check per transaction reserved.
#[test]
fn sequencer_get_non_preferred_blobs_releases_deferred_escrow() {
    let (
        TestData {
            regular_sequencer, ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();
    defer_two_tx_batch(&regular_sequencer, &mut runner);

    let (gas_price, reserved_gas) = replay_deferred_blobs_as_preferred_sequencer(&mut runner);

    let two_checks = <S as GasSpec>::max_tx_check_costs()
        .checked_scalar_product(2)
        .unwrap()
        .checked_value(gas_price)
        .unwrap();
    assert_eq!(
        reserved_gas,
        vec![Some(two_checks)],
        "The deferred blob should be selected with the gas for its two pre-execution checks, moved out of the deferred escrow"
    );
}

/// The preferred sequencer must reserve exactly as much gas for a deferred blob as the node takes
/// out of the blob's escrow when it executes the same rollup block. The sequencer's state is later
/// overwritten with the node's, so a mismatch that still lets the blob's transactions execute
/// would otherwise go unnoticed.
#[test]
fn sequencer_reserves_the_same_gas_for_a_deferred_blob_as_the_node() {
    // What the node does: the preferred batch advances the visible slot by one, over the deferred
    // blob.
    let (
        TestData {
            preferred_sequencer,
            regular_sequencer,
            ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();
    let sender_address = regular_sequencer.user_info.address();
    let sender_balance = |runner: &crate::TestRunner<_>| {
        runner.query_visible_state(|state| {
            crate::TestRunner::<SoftConfRT>::bank_gas_balance(&sender_address, state)
                .unwrap_or(Amount::ZERO)
        })
    };
    let escrowed_on_node = defer_two_tx_batch(&regular_sequencer, &mut runner);
    let sender_balance_before = sender_balance(&runner);
    let blobs = build_soft_confirmation_blobs(
        &vec![(
            preferred_sequencer.clone(),
            SequencerInfo::Preferred {
                slots_to_advance: 1,
                sequence_number: 1,
            },
        )],
        &mut HashMap::new(),
        0,
    );
    runner.execute::<RelevantBlobs<MockBlob>>(blobs);
    assert_eq!(
        runner.receipts().last().unwrap().batch_receipts.len(),
        2,
        "The preferred batch and the deferred batch must both execute on the node"
    );
    // The node keeps the gas for the pre-execution checks and refunds the rest of the escrow to
    // the sender's bank balance, which nothing else touches in this block.
    let refunded_by_node = sender_balance(&runner)
        .checked_sub(sender_balance_before)
        .unwrap();
    let reserved_by_node = escrowed_on_node.checked_sub(refunded_by_node).unwrap();

    // What the preferred sequencer does, starting from an identical state.
    let (
        TestData {
            regular_sequencer, ..
        },
        mut runner,
    ) = setup_with_registration_soft_confirmation_kernel();
    let escrowed_on_sequencer = defer_two_tx_batch(&regular_sequencer, &mut runner);
    assert_eq!(
        escrowed_on_sequencer, escrowed_on_node,
        "Both runs must start from the same escrow to be comparable"
    );
    let (_, reserved_by_sequencer) = replay_deferred_blobs_as_preferred_sequencer(&mut runner);

    assert_eq!(
        reserved_by_sequencer,
        vec![Some(reserved_by_node)],
        "The preferred sequencer must reserve the same gas for the deferred blob as the node"
    );
}
