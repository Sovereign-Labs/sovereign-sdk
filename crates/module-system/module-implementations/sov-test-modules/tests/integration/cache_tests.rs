use sov_test_modules::cache_module::{CacheAndRevertTester, CallMessage, Event, TestAndSet};
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_zk_runtime, AsUser, SlotInput, TestSpec, TestUser, TransactionTestCase,
};

generate_zk_runtime!(TestRuntime <= test_module: CacheAndRevertTester<S>);

type S = TestSpec;
type RT = TestRuntime<S>;

#[allow(clippy::type_complexity)]
fn setup() -> (TestRunner<TestRuntime<S>, S>, TestUser<S>, TestUser<S>) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(2);

    let admin_account = genesis_config.additional_accounts()[0].clone();
    let extra_account = genesis_config.additional_accounts()[1].clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.clone().into(), ());

    (
        TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default()),
        admin_account,
        extra_account,
    )
}

// Test that the cache works as expected in that...
// - Values can be set, retrieved, and deleted
// - Values are shared across batches in the same slot
// - The cache is cleared at the end of each slot
//
// We use a number of different types to ensure that ops on different types don't interfere with each other
#[test]
fn test_setting_and_getting_values() {
    let (mut runner, admin, _) = setup();
    // First, we'll create two batches running various cache operations
    let first_batch_ops = vec![
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetU8(
            TestAndSet {
                expected_value: None, // Insert a new u8. Cache should have been empty
                new_value: Some(1),
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetU8(
            TestAndSet {
                expected_value: Some(1), // Update the u8. Cache should have remembered the old value
                new_value: Some(2),
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetU16(
            TestAndSet {
                expected_value: None, // Insert a new u16. no u16 should be cached
                new_value: Some(11),
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetString(
            TestAndSet {
                new_value: Some("hello".try_into().unwrap()), // Insert a new string. No string should be cached
                expected_value: None,
            },
        )),
    ];
    let second_batch_ops = vec![
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetString(
            TestAndSet {
                expected_value: Some("hello".try_into().unwrap()), // Update the string. The old value should be cached from the previous batch
                new_value: Some("goodbye".try_into().unwrap()),
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetString(
            TestAndSet {
                expected_value: Some("goodbye".try_into().unwrap()), // Delete the string. The old value should be cached from the previous tx
                new_value: None,
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetString(
            TestAndSet {
                expected_value: None, // Insert a new string. No string should be cached
                new_value: Some("hello again".try_into().unwrap()),
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetString(
            TestAndSet {
                expected_value: Some("hello again".try_into().unwrap()),
                new_value: Some("hello again".try_into().unwrap()), // Update the string to itself. Nothing weird should happen
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetString(
            TestAndSet {
                expected_value: Some("hello again".try_into().unwrap()), // Ensure that the string still has the original value. Update it again
                new_value: Some("goodbye again".try_into().unwrap()),
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetU16(
            TestAndSet {
                expected_value: Some(11),
                new_value: Some(11), // Check that the u16 is still cached. Leave it where it is
            },
        )),
        admin.create_plain_message::<RT, CacheAndRevertTester<S>>(CallMessage::TestAndSetU8(
            TestAndSet {
                expected_value: Some(2), // Delete the u8. The old value should be cached from the previous batch
                new_value: None,
            },
        )),
    ];

    // Second, execute the two batches
    let num_operations_first_batch = first_batch_ops.len();
    let num_operations_second_batch = second_batch_ops.len();
    let output = runner
        .execute(SlotInput::Batches(vec![
            first_batch_ops.into(),
            second_batch_ops.into(),
        ]))
        .0;

    // Third, assert each batch executed successfully
    let first_batch_receipt = output.batch_receipts[0].clone();
    assert_eq!(
        first_batch_receipt.tx_receipts.len(),
        num_operations_first_batch
    );
    for receipt in first_batch_receipt.tx_receipts {
        assert!(receipt.receipt.is_successful());
    }

    let second_batch_receipt = output.batch_receipts[1].clone();
    assert_eq!(
        second_batch_receipt.tx_receipts.len(),
        num_operations_second_batch
    );
    for receipt in second_batch_receipt.tx_receipts {
        assert!(receipt.receipt.is_successful());
    }

    // Finally, check that the cache has been cleared.
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
            CallMessage::TestAndSetString(TestAndSet {
                expected_value: None,
                new_value: None,
            }),
        ),
        assert: Box::new(|output, _| {
            assert!(output.tx_receipt.is_successful());
        }),
    });
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
            CallMessage::TestAndSetU16(TestAndSet {
                expected_value: None,
                new_value: None,
            }),
        ),
        assert: Box::new(|output, _| {
            assert!(output.tx_receipt.is_successful());
        }),
    });
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
            CallMessage::TestAndSetU8(TestAndSet {
                expected_value: None,
                new_value: None,
            }),
        ),
        assert: Box::new(|output, _| {
            assert!(output.tx_receipt.is_successful());
        }),
    });
}

#[test]
// Test that reverted txs don't modify the cache
fn test_reverted_txs_dont_modify_cache() {
    let (mut runner, admin, _) = setup();
    let output = runner.execute(SlotInput::Batch(
        vec![
            admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
                CallMessage::SetAndRevertString(Some("hello".try_into().unwrap())),
            ), // Set a value, but then revert the tx
            admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
                CallMessage::TestAndSetString(TestAndSet {
                    expected_value: None, // Assert that the value is not cached, then insert an entry
                    new_value: Some("hello".try_into().unwrap()),
                }),
            ),
            admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
                CallMessage::SetAndRevertString(None),
            ), // Delete the value, but then revert the tx
            admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
                CallMessage::TestAndSetString(TestAndSet {
                    expected_value: Some("hello".try_into().unwrap()), // Assert that the value is still cached
                    new_value: None,
                }),
            ),
        ]
        .into(),
    ));
    let receipt = output.0.batch_receipts[0].clone();
    assert_eq!(receipt.tx_receipts.len(), 4);
    assert!(receipt.tx_receipts[0].receipt.is_reverted());
    assert!(receipt.tx_receipts[1].receipt.is_successful());
    assert!(receipt.tx_receipts[2].receipt.is_reverted());
    assert!(receipt.tx_receipts[3].receipt.is_successful());
}

/// Tests that updates to the cache are not affected by the "undo" capability of tx state if undo is not triggered and
/// that "undo" does revert cache changes.
#[test]
fn test_set_and_maybe_undo_cache_behavior() {
    let (mut runner, admin, _) = setup();
    let output = runner.execute(SlotInput::Batch(
        vec![
            admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
                // Setup: Set a value in cache
                CallMessage::TestSetAndMaybeUndo {
                    cache_value: TestAndSet {
                        expected_value: None,
                        new_value: Some("hello".try_into().unwrap()),
                    },
                    state_value: 1,
                    undo: false,
                },
            ),
            admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
                // Test and set that value, but then undo the change in user space. This tx should not revert but should have no effect on the cache
                CallMessage::TestSetAndMaybeUndo {
                    cache_value: TestAndSet {
                        expected_value: Some("hello".try_into().unwrap()),
                        new_value: None,
                    },
                    state_value: 1,
                    undo: true,
                },
            ),
            admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
                // Test and set the cache value again. This tx should be successful because the previous one did not modify the cache
                CallMessage::TestSetAndMaybeUndo {
                    cache_value: TestAndSet {
                        expected_value: Some("hello".try_into().unwrap()),
                        new_value: None,
                    },
                    state_value: 2,
                    undo: false,
                },
            ),
        ]
        .into(),
    ));

    let receipt = output.0.batch_receipts[0].clone();
    assert_eq!(receipt.tx_receipts.len(), 3);
    assert!(receipt.tx_receipts[0].receipt.is_successful());
    assert!(receipt.tx_receipts[1].receipt.is_successful());
    assert!(receipt.tx_receipts[2].receipt.is_successful());
}

/// Tests that functionality for user-space undo does not prevent values from being written or events from being emitted unless
/// "undo" is triggered.
#[test]
fn test_set_and_maybe_undo_state_and_event_behavior() {
    let (mut runner, admin, _) = setup();
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
            // Write a value to state and emit an event. Assert that they go through
            CallMessage::TestSetAndMaybeUndo {
                cache_value: TestAndSet {
                    expected_value: None,
                    new_value: None,
                },
                state_value: 1,
                undo: false,
            },
        ),
        assert: Box::new(|output, state| {
            // Check that the output is here
            assert!(output.tx_receipt.is_successful());
            assert_eq!(output.events.len(), 1);
            assert_eq!(
                output.events[0],
                TestRuntimeEvent::TestModule(Event::SetValue(1))
            );
            let module = CacheAndRevertTester::<S>::default();
            assert_eq!(module.value.get(state).unwrap(), Some(1));
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, CacheAndRevertTester<S>>(
            // Write a value to state and emit an event, then undo them. The tx should not revert but the state and event should not be written
            CallMessage::TestSetAndMaybeUndo {
                cache_value: TestAndSet {
                    expected_value: None,
                    new_value: None,
                },
                state_value: 2,
                undo: true,
            },
        ),
        assert: Box::new(|output, state| {
            // The tx should be successful, but no events should be emitted and the state should not be modified
            assert!(output.tx_receipt.is_successful());
            assert_eq!(output.events.len(), 0);
            let module = CacheAndRevertTester::<S>::default();
            assert_eq!(module.value.get(state).unwrap(), Some(1));
        }),
    });
}
