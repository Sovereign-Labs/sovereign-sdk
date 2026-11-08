use super::*;

#[derive(Debug, serde::Deserialize)]
struct ManyValuesItemResponse {
    #[allow(unused)]
    index: u64,
    value: Option<u8>,
}

async fn wait_for_many_values_item(
    test_rollup: &TestRollup<TestBlueprint>,
    da_layer: &mut DaLayerWithSubscription,
    index: u64,
    expected_value: u8,
    max_blocks_to_produce: usize,
) -> u8 {
    let endpoint = format!("/modules/value-setter/state/many-values/items/{index}");
    let mut last_seen_value: Option<Option<u8>> = None;
    let mut last_error: Option<String> = None;

    for attempt in 0..=max_blocks_to_produce {
        match test_rollup
            .client
            .query_rest_endpoint::<ManyValuesItemResponse>(&endpoint)
            .await
        {
            Ok(response) => {
                last_seen_value = Some(response.value);
                if response.value == Some(expected_value) {
                    return expected_value;
                }
            }
            Err(error) => {
                last_error = Some(error.to_string());
            }
        }

        if attempt == max_blocks_to_produce {
            break;
        }

        da_layer.produce_block().await.unwrap();
        test_rollup.wait_for_node_synced().await.unwrap();
    }

    panic!(
        "Timed out waiting for many_values[{index}] to become {expected_value}. Last seen value: {last_seen_value:?}, last error: {last_error:?}"
    );
}

const SEQUENCER_RECOVERY_ERROR: &str = "The preferred sequencer is recovering from downtime and cannot provide soft-confirmations at this time";

#[tokio::test(flavor = "multi_thread")]
async fn seq_behind_deferred_slots_count_simple_lagging() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "40");
    let (test_rollup, admin) = create_test_rollup().await;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();

    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;

    // Sanity check tx that the rollup works
    test_rollup.submit(&admin, set_value(8)).await.unwrap();

    tracing::info!("Producing DA blocks for inclusion sanity check tx inclusion");
    da_layer.produce_and_wait_for_n_slots(10).await;

    // Pause sequencer update_state and run some blocks so deferred_slots_count is reached
    test_rollup.pause_preferred_batches().await;
    tracing::info!("Preferred sequencer batch production paused.");

    // Preferred sequencer should accept the transaction (and keep it as an in-progress batch since
    // we've stopped update_state and thus aren't producing batches)
    const UPDATE_TWO_VALUE: u64 = 19;
    test_rollup
        .submit(&admin, set_value(UPDATE_TWO_VALUE))
        .await
        .unwrap();

    tracing::info!(
        "Producing subsequent DA blocks while sequencer is paused, to exceed deferred_slots_count"
    );
    // This can be lower than DEFERRED_SLOTS_COUNT because the sequencer takes into account a)
    // possible node lag and b) a 90% threshold.
    for _ in 0..30 {
        let _ = da_layer.produce_block().await; // Don't wait for state updates since we've just paused them
    }
    // Make sure the DA has synced everything
    test_rollup.wait_for_node_synced().await.unwrap();

    tracing::info!("Resuming preferred sequencer batch production.");
    // Subscribe to state update notifications BEFORE resuming, so we don't miss the
    // recovery notification. This channel bypasses the state updator message queue
    // (which blocks during trigger_recovery's async calls under CPU pressure).
    let mut recovery_sub = test_rollup.subscribe_state_updates().await.unwrap();
    test_rollup.resume_preferred_batches().await;
    // Produce one block to trigger the state update that detects the gap.
    test_rollup.da_service.produce_block_now().await.unwrap();
    // Wait for the notification that indicates recovery was triggered.
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let notification = recovery_sub.next().await.unwrap().unwrap();
            if notification.triggered_recovery {
                break;
            }
        }
    })
    .await
    .expect("Timed out waiting for sequencer to enter recovery");

    // Create transaction that should fail: sequencer should not accept transactions while in
    // recovery.
    // We use set_many_values so we don't overwrite the value from set_value earlier, so we can
    // check that both had an effect by querying the separate state items.
    const UPDATE_VEC_VALUE: u8 = 12;
    let tx_update_vec = tx_set_many_values(&admin.private_key, 2, vec![UPDATE_VEC_VALUE]);
    tracing::info!("Trying to send transaction during recovery - expecting rejection");
    let err = client
        .send_raw_tx_to_sequencer(&tx_update_vec)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(SEQUENCER_RECOVERY_ERROR),
        "Expected recovery error, got: {err}"
    );

    // Give time for the sequencer to catch up its visible state number
    tracing::info!("Producing DA blocks to let the sequencer resync.");
    tokio::time::timeout(TestRollup::<TestBlueprint>::POLLING_TIMEOUT, async {
        while !test_rollup.is_sequencer_ready().await {
            let _ = da_layer.produce_block().await;
            sleep(Duration::from_millis(50)).await; // Notifications don't work during recovery.
        }
    })
    .await
    .expect("Sequencer did not become ready while producing DA blocks");
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Submit the same transaction to the now-working sequencer
    // This transaction will be soft-confirmed. The assertion should pass at this stage.
    client
        .send_raw_tx_to_sequencer(&tx_update_vec)
        .await
        .unwrap();

    tracing::info!(
        "Producing blocks until the post-recovery tx is visible in state (deterministic inclusion)"
    );
    let actual_many_value =
        wait_for_many_values_item(&test_rollup, &mut da_layer, 0, UPDATE_VEC_VALUE, 80).await;
    test_rollup.wait_for_node_synced().await.unwrap();

    // Assert that the earlier transactions sent just before the sequencer went into recovery was
    // flushed and processed by the node
    let response = test_rollup
        .client
        .query_rest_endpoint::<ValueResponse>("/modules/value-setter/state/value")
        .await
        .unwrap();
    let actual_value = response.value;
    assert_eq!(
        actual_value, UPDATE_TWO_VALUE as u32,
        "Expected value to be {UPDATE_TWO_VALUE}, but got {actual_value}"
    );

    // Assert that the transaction sent after the sequencer exited recovery was processed (i.e.
    // that the rollup is now functional)
    assert_eq!(
        actual_many_value, 12u8,
        "Expected many_values[0] to be 12, but got {actual_many_value}"
    );

    tracing::info!("All asserts successful, shutting down rollup");
    tokio::time::timeout(TEST_NORMAL_SHUTDOWN_TIMEOUT, test_rollup.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_behind_deferred_slots_count_with_shutdown() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "40");
    let (test_rollup, admin) = create_test_rollup().await;

    // Finalise some blocks
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(8).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Sanity check tx that the rollup works
    test_rollup.submit(&admin, set_value(8)).await.unwrap();

    tracing::info!("Producing DA blocks for inclusion sanity check tx inclusion");
    da_layer.produce_and_wait_for_n_slots(10).await;

    // Send a transaction that will be queued while rollup is shut down
    const UPDATE_TWO_VALUE: u64 = 19;
    test_rollup
        .submit(&admin, set_value(UPDATE_TWO_VALUE))
        .await
        .unwrap();

    // Keep a reference to the DA service before shutting down
    let da_service = test_rollup.da_service.clone();

    // Shutdown the rollup while preserving the DA layer
    tracing::info!("Shutting down rollup to test restart behavior");
    let builder = test_rollup.shutdown().await.unwrap();

    tracing::info!("Producing DA blocks while rollup is shut down, to exceed deferred_slots_count");
    // This can be lower than DEFERRED_SLOTS_COUNT because the sequencer takes into account
    // a) possible node lag
    // b) a 90% threshold.
    da_service.produce_n_blocks_now(30).await.unwrap();

    // Restart the rollup
    tracing::info!("Restarting rollup after exceeding deferred_slots_count");
    let test_rollup = builder.start().await.unwrap();
    let client = test_rollup.api_client().clone();
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    // Subscribe BEFORE node sync so we don't miss the recovery notification
    // (recovery may trigger during the sync phase itself).
    let mut recovery_sub = test_rollup.subscribe_state_updates().await.unwrap();

    // Sync the node to the new DA blocks. Recovery may trigger during sync.
    test_rollup.wait_for_node_synced().await.unwrap();
    // Produce one more block in case recovery hasn't triggered yet.
    test_rollup.da_service.produce_block_now().await.unwrap();
    // Wait for the notification that indicates recovery was triggered. This
    // bypasses the state updator (which blocks during trigger_recovery under load).
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let notification = recovery_sub.next().await.unwrap().unwrap();
            if notification.triggered_recovery {
                break;
            }
        }
    })
    .await
    .expect("Timed out waiting for sequencer to enter recovery");

    // Create transaction that should fail: sequencer should not accept transactions while in recovery
    const UPDATE_VEC_VALUE: u8 = 12;
    let tx_update_vec = tx_set_many_values(&admin.private_key, 2, vec![UPDATE_VEC_VALUE]);
    tracing::info!("Trying to send transaction during recovery - expecting rejection");
    let err = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_vec),
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains(SEQUENCER_RECOVERY_ERROR),
        "Expected recovery error, got: {err}"
    );

    // Give time for the sequencer to catch up its visible state number
    tracing::info!("Producing DA blocks to let the sequencer resync.");
    tokio::time::timeout(TestRollup::<TestBlueprint>::POLLING_TIMEOUT, async {
        while !test_rollup.is_sequencer_ready().await {
            test_rollup.da_service.produce_block_now().await.unwrap();
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Sequencer did not become ready while producing DA blocks");
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Submit the same transaction to the now-working sequencer
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx_update_vec),
        })
        .await
        .unwrap();

    tracing::info!(
        "Producing blocks until the post-recovery tx is visible in state (deterministic inclusion)"
    );
    let actual_many_value =
        wait_for_many_values_item(&test_rollup, &mut da_layer, 0, UPDATE_VEC_VALUE, 80).await;
    test_rollup.wait_for_node_synced().await.unwrap();

    // Assert that the earlier transaction sent before shutdown was processed
    let response = test_rollup
        .client
        .query_rest_endpoint::<ValueResponse>("/modules/value-setter/state/value")
        .await
        .unwrap();
    let actual_value = response.value;
    assert_eq!(
        actual_value, UPDATE_TWO_VALUE as u32,
        "Expected value to be {UPDATE_TWO_VALUE}, but got {actual_value}"
    );

    // Assert that the transaction sent after recovery was processed
    assert_eq!(
        actual_many_value, 12u8,
        "Expected many_values[0] to be 12, but got {actual_many_value}"
    );

    tracing::info!("All asserts successful, shutting down rollup");
    test_rollup.shutdown().await.unwrap();
}

/// Test that the sequencer can compute state roots for itself to avoid panics.
///
/// This test works by causing the node to fall far behind the sequencer in processing rollup blocks.
/// This is done by preventing the sequencer batches from being included on DA, while still producing lots of DA blocks.
///
/// Since there are always new finalized blocks available,
/// the sequencer will happily create new rollup blocks far in advance of the node,
/// triggering the case we care about.
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_state_root_computation_when_blobs_are_delayed() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_STATE_ROOT_DELAY_BLOCKS", "1");
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
            (),
            PaymasterConfig::default(),
            (),
            (),
        );
    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = tempdir_inside_codebase_dir();

    let test_rollup = SeqTestRollup::<TestRuntime<TestSpec>>::new(
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
    )
    .dir(dir.clone())
    .block_producing_config(BlockProducingConfig::OnAnySubmit {
        block_wait_timeout_ms: None,
    })
    .start()
    .await;

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    test_rollup
        .tenderly_produce_blocks((TEST_FINALIZATION_BLOCKS + 2) as usize)
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();
    let blocks_to_delay: u64 = 100;
    test_rollup
        .da_service
        .set_delay_blobs_by(blocks_to_delay as u32)
        .await;

    let mut slot_subscription = test_rollup.api_client().subscribe_slots().await.unwrap();
    // In this loop, transactions should be accepted, but node sees empty DA blocks, because current blobs are delayed
    for i in 0..blocks_to_delay {
        test_rollup.wait_for_node_synced().await.unwrap();
        test_rollup.submit(&admin, set_value(i)).await.unwrap();
        test_rollup.da_service.produce_block_now().await.unwrap();
        let _slot = next_within(&mut slot_subscription, NOTIFICATION_TIMEOUT)
            .await
            .unwrap();
    }
    // Now almost all blobs are still delayed, only first has landed

    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let acceptable_block_processing_time = std::time::Duration::from_millis(500);

    let total_waiting_time = acceptable_block_processing_time * blocks_to_delay as u32;

    // Producing more blocks, so all delayed blobs are flushed.
    test_rollup
        .da_service
        .produce_n_blocks_now(blocks_to_delay as usize)
        .await
        .unwrap();
    if let Some(sleep_time) =
        TestRollup::<TestBlueprint>::POLLING_TIMEOUT.checked_sub(total_waiting_time)
    {
        // Wait for delayed blobs to age through the configured delay window before checking sync.
        tokio::time::sleep(sleep_time).await;
    }
    test_rollup.wait_for_node_synced().await.unwrap();

    let response = test_rollup
        .client
        .query_rest_endpoint::<ValueResponse>("/modules/value-setter/state/value")
        .await
        .unwrap();
    let actual_value = response.value;
    // We could've checked the actual value, but that's not the point of this test.
    println!("actual value in the end: {actual_value:?}");

    tokio::time::timeout(TEST_NORMAL_SHUTDOWN_TIMEOUT, test_rollup.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_no_crashes_on_resync_with_transactions() {
    let (test_rollup, admin) = create_test_rollup().await;

    let (test_rollup, state) = setup_test_rollup_with_initial_state(test_rollup, &admin).await;

    let actions = vec![
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::NewDaSlot,
        TestingAction::Sleep { duration_ms: 100 },
    ];
    let (test_rollup, state) =
        run_actions_against_test_rollup(actions, test_rollup, &admin.clone(), state).await;

    let actions = vec![
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::NewDaSlot,
        TestingAction::Sleep { duration_ms: 100 },
    ]
    .into_iter()
    .cycle()
    .take(160)
    .collect(); // repeat for 40 blocks
    let (test_rollup, state) =
        run_actions_against_test_rollup(actions, test_rollup, &admin.clone(), state).await;

    let builder = test_rollup.shutdown().await.unwrap();

    let rollup_storage_path = builder.storage_path();
    // Next, delete everything except the preferred sequencer DB. Resync again to verify that this
    // doesn't interfere.
    // NOMT persists state in these directories:
    // - user_nomt_db, kernel_nomt_db (NOMT state)
    // - state-db, archival-state-db (FlatStateDb)
    // - accessory, ledger, blob_sender (common to both)
    for path in [
        "user_nomt_db",
        "kernel_nomt_db",
        "state-db",
        "archival-state-db",
        "accessory",
        "ledger",
        "blob_sender",
    ] {
        let full_path = rollup_storage_path.path().join(path);
        if full_path.exists() {
            std::fs::remove_dir_all(full_path).unwrap();
        }
    }

    let test_rollup = builder.start().await.unwrap();

    // Wait deterministically for both components after restart instead of using a fixed delay.
    test_rollup.wait_for_node_synced().await.unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Verify accepting actions works
    let actions = vec![
        TestingAction::AcceptTx,
        TestingAction::AcceptTx,
        TestingAction::NewDaSlot,
        TestingAction::Sleep { duration_ms: 100 },
        TestingAction::QuerySetValue,
    ];

    let (test_rollup, _state) =
        run_actions_against_test_rollup(actions, test_rollup, &admin.clone(), state).await;

    test_rollup.shutdown().await.unwrap();
}

/// This test checks our "nuke the queue" functionality, which ensures fairness when the sequencer has downtime.
///
/// Recall that some transaction types have a "speedbump":
/// where their handlers sleep for a short period of time **before entering** the execution queue.
/// If the sequencer has downtime during that sleep,
/// these transactions need to be rejected for safety - otherwise, they might "time travel"
/// and be executed before other transactions that arrived earlier -
/// since those transactions were rejected during the downtime.
/// This test covers that functionality.
///
/// This test works by...
/// - Sending a tx that has to go through the speedbump
/// - Intentionally sending enough transactions to cause downtime while that delayed tx is waiting on the speedbump
/// - Producing blocks so that the sequencer recovers from the downtime
/// - Ensuring that the delayed tx still fails with a 503
/// -----
/// Correctness of this test can be validated by disabling queue id check:
///  - https://github.com/Sovereign-Labs/sovereign-sdk/blob/e921a2bc5a395f807b73ccda2f41dac74c2a1b8e/crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs#L739
#[tokio::test(flavor = "multi_thread")]
async fn flaky_txs_that_enter_before_downtime_are_dropped() {
    // Set a small batch time limit to ensure that the sequencer will be overloaded after the first tx.
    // Question from Nikolai: Why it is going to be overloaded?
    let block_execution_time_ms = 1000;
    assert!(
        block_execution_time_ms * 2 < DELAYED_TX_DELAY_MS,
        "Test relies on delay being larger than 2 max batch times"
    );
    let (test_rollup, admin) =
        create_test_rollup_with(|b| b.max_batch_execution_time_millis(block_execution_time_ms))
            .await;

    let assert_response_is_overloaded = |response: Result<
        ResponseValue<TxInfoWithConfirmation>,
        Error<ApiError>,
    >|
     -> anyhow::Result<()> {
        let error = response
            .err()
            .ok_or(anyhow::anyhow!("Expected error response, got success"))?
            .to_string();
        if !error.contains("The sequencer is temporarily overloaded") {
            anyhow::bail!(
                "Expected error to contain 'The sequencer is temporarily overloaded', got: {error}"
            )
        }
        Ok(())
    };

    // +5 to be sure
    test_rollup
        .tenderly_produce_blocks(TEST_FINALIZATION_BLOCKS as usize + 5)
        .await
        .unwrap();
    test_rollup.wait_for_node_synced().await.unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();

    let max_attempts = 10;
    let mut results = Vec::with_capacity(max_attempts);
    let time_for_seq_to_accept_and_delay = std::time::Duration::from_millis(100);

    let mut generation = 1;
    // On each loop we send DelayedCallMsg and SetValueAndSleep and save their results in a vector.
    // Delayed sent first, SetValueAndSleep shortly after.
    // If SetValueAndSleep succeeds, it means that sequencer still has finalized slots.
    // In this case, DelayedCallMsg will be accepted first thing in the *next* batch without affecting it much.
    // If SetValueAndSleep fails, we stop loop, assuming it triggered overload.
    // Then after loop we check:
    //  * The first iteration should always succeed: that's baseline that sequencer is ok.
    //  * The last iteration should fail with particular error: sequencer got overloaded

    // High level picture of the last finalized slot:
    // 1. 0ms: accept DelayedMsg -> delayed
    // 2. 100ms: accept SetValueAndSleep -> starts sleeping
    // 3. 1201ms: SetValueAndSleep completed, but executor detects batch went over time, closing the batch.
    // 4. Now it detects there's no more finalized slots available, enters "downtime".
    //    It clears the queue and rejects all messages that were sleeping. This means that DelayedMessage is rejected straight away
    // 5. Produce more blocks to create finalized slots.
    // 6. 2500ms If the queue had not been cleared, DelayedMsg would wake up and be executed.
    for _ in 0..max_attempts {
        let set_value_and_sleep = tx_set_value_and_sleep(
            &admin.private_key,
            generation,
            0,
            block_execution_time_ms + 200,
        );
        generation += 1;
        let delayed_tx = tx_delayed_call(&admin.private_key, generation);
        // Delayed tx will be either accepted in the next batch or rejected after downtime
        let delayed_tx_handle = tokio::spawn({
            let client = client.clone();
            async move { client.send_raw_tx_to_sequencer(&delayed_tx).await }
        });
        // Allow the delayed transaction to enter its speedbump path before submitting the second tx.
        tokio::time::sleep(time_for_seq_to_accept_and_delay).await;
        let set_value_and_sleep_result =
            client.send_raw_tx_to_sequencer(&set_value_and_sleep).await;
        let is_err = set_value_and_sleep_result.is_err();
        results.push((set_value_and_sleep_result, delayed_tx_handle));
        if is_err {
            break;
        }
    }
    // There should be at least 1 success and 1 error
    assert!(results.len() >= 2);
    let results_sent = results.len();
    let (the_first_set_and_sleep_response, the_first_delayed) = results.remove(0);
    the_first_set_and_sleep_response.expect("first etValueAndSleep should always succeed");
    the_first_delayed
        .await
        .expect("first DelayedMsg is panicked")
        .expect("first DelayedMsg should always succeed");
    let (the_last_set_and_sleep_response, the_last_delayed) = results.pop().unwrap();

    assert_response_is_overloaded(the_last_set_and_sleep_response)
        .expect("last SetValueAndSleep should be rejected due to overload");

    // Resuming normal operation: producing more blocks:
    // 1 for each submitted batch
    // 2 for buffer and
    // Another interesting point: both batches landed in single block.
    // How visible slot height is going to be increased?
    test_rollup
        .tenderly_produce_blocks(results_sent + 2)
        .await
        .unwrap();
    test_rollup.wait_for_node_synced().await.unwrap();

    let the_last_delayed_response = the_last_delayed.await.expect("last DelayedMsg is panicked");
    assert_response_is_overloaded(the_last_delayed_response)
        .expect("last DelayedMsg should be rejected rejected due to overload");

    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let last_tx = tx_set_value_and_sleep(
        &admin.private_key,
        generation,
        31337,
        block_execution_time_ms + 200,
    );
    client
        .send_raw_tx_to_sequencer(&last_tx)
        .await
        .expect("Sequencer is not operating normally");
}

/// This test checks what happens when the DA layer takes a long time to publish blobs under load.
/// This is a regression test for behavior which would cause the sequencer to produce ever-larger batches
/// and then blow up.
///
/// If this test is flaky, it can be made more reliable by simply increasing the `worker_timeout_secs` variable.
/// This makes the test take longer to run, but also reduces the chance of it flaking.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "Takes ~90s; runs in the nightly workflow"]
async fn heavy_blob_submission_long_delay() {
    let worker_timeout_secs = 90;
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "150000");
    let max_batch_size = 1 << 30;
    let blob_processing_timeout_secs = 500;
    let (task_completed_sender, task_completed_receiver) = tokio::sync::oneshot::channel();
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config.additional_accounts()[0].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
            (),
            PaymasterConfig::default(),
            (),
            (),
        );
    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = Arc::new(tempfile::tempdir().unwrap());

    let test_rollup = SeqTestRollup::<TestRuntime<TestSpec>>::new(
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
    )
    .dir(dir)
    .max_batch_size_bytes(max_batch_size)
    // Block time is intentionally low to put extra pressure on Node and Sequencer.
    .block_producing_config(BlockProducingConfig::Periodic { block_time_ms: 200 })
    .blob_processing_timeout_secs(blob_processing_timeout_secs)
    // Set the batch time limit to twice the block time
    .max_batch_execution_time_millis(400)
    .start()
    .await;

    test_rollup.wait_for_sequencer_ready().await.unwrap();
    test_rollup.da_service.set_delay_blobs_by(30).await;

    let timeout_handle = tokio::spawn(async move {
        let timeout = worker_timeout_secs + 20;
        tokio::time::timeout(Duration::from_secs(timeout), task_completed_receiver)
            .await
            .unwrap()
    });

    let nonce = Arc::new(AtomicU64::new(0));

    let workers_timeout = Duration::from_secs(worker_timeout_secs);
    // Spawn 20 workers to spam the sequencer with transactions.
    let spam_start = std::time::Instant::now();
    let workers = (0..50)
        .map(|_| {
            let client = test_rollup.api_client().clone();
            let nonce = nonce.clone();
            let key = admin.private_key.clone();
            let nonce = nonce.clone();
            tokio::spawn(async move {
                let start = std::time::Instant::now();
                let mut success = 0;
                let mut errors = 0;
                loop {
                    if start.elapsed() > workers_timeout {
                        break;
                    }
                    let generation = nonce.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let tx = tx_set_many_values(&key, generation, vec![generation as u8; 1024]);

                    let resp = client.send_raw_tx_to_sequencer(&tx).await;
                    if resp.is_err() {
                        errors += 1;
                    } else {
                        success += 1;
                    }
                }
                (success, errors)
            })
        })
        .collect::<Vec<_>>();

    // Wait for the workers to finish.
    let spam_duration = spam_start.elapsed();
    let results = futures::future::join_all(workers).await;
    let mut total_success = 0;
    let mut total_errors = 0;
    for result in results {
        let (successes, errors) = result.unwrap();
        total_success += successes;
        total_errors += errors;
    }

    println!("Success requests {total_success}, error requests {total_errors}");

    tokio::select! {
        _ = timeout_handle => {
            panic!("Test timed out! This means the sequencer has regressed! Spam took {spam_duration:?}. Success requests {total_success}, error requests {total_errors}");
        }
        shutdown_result = test_rollup.shutdown() => {
            shutdown_result.unwrap();
            task_completed_sender.send(()).expect("Failed to send task completed signal");
        }
    }
}
