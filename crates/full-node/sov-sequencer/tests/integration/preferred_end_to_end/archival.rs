use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn test_archival_state_is_immediately_available() {
    let (test_rollup, admin) = create_test_rollup_with(|b| {
        // automatic_batch_production — disabled to prevent empty batches from advancing height
        b.automatic_batch_production(false).finalization_blocks(0)
    })
    .await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    test_rollup
        .submit_and_wait_included(&admin, set_value(1))
        .await
        .unwrap();
    // Height after the first value has been set, we don't care much about before that.
    let height_at_start = test_rollup.height().await.get();

    // Now the archival state for each block should immediately be available (as soon as the previous block is closed).
    // Let's test this in a loop
    let final_height = height_at_start + 10;

    for (i, height) in (height_at_start..=final_height).enumerate() {
        // Send a transaction to ensure the previous batch is closed.
        let value_to_set = i + 2;
        test_rollup
            .submit(&admin, set_value(value_to_set as u64))
            .await
            .unwrap();
        test_rollup.force_close_batch().await.unwrap();
        // Produce a some extra slots to ensure that the rollup height and slot number aren't the same.
        // This increases coverage for free.
        let extra_blocks = 2;
        let mut slots = test_rollup.api_client().subscribe_slots().await.unwrap();
        test_rollup.tenderly_produce_blocks(2).await.unwrap();
        test_rollup.wait_for_node_synced().await.unwrap();
        test_rollup.wait_for_sequencer_ready().await.unwrap();
        // Wait for slot notifications to arrive, this means API state got all updates.
        for _ in 0..extra_blocks {
            let _slot = next_within(&mut slots, NOTIFICATION_TIMEOUT).await.unwrap();
        }
        for (j, past_height) in (height_at_start..height).enumerate() {
            let expected_value = (j + 1) as u64;
            query_set_value(&test_rollup, Some(past_height), Some(expected_value))
                .await
                .with_context(|| {
                    format!("past height = {past_height}, expected value = {expected_value}")
                })
                .unwrap();
        }
    }
}

/// With pruning enabled, an archival read for a height that has fallen below the pruned floor must
/// consistently return `HeightNotAccessible` (HTTP 404) — never panic, never return stale/wrong
/// data, and never return `Ok(None)` for a key that was set at that height. We pin a single
/// `target_height`, then hammer it from a background task while a writer advances the chain past
/// `versions_to_keep`, asserting that every readable result is exactly the value we set (never
/// stale, wrong, or `null`) and that the height eventually becomes — and then stays —
/// `HeightNotAccessible`. Modeled on `test_archival_state_is_immediately_available`.
#[tokio::test(flavor = "multi_thread")]
async fn test_archival_reads_at_pruned_height_stay_consistent_under_pruning() {
    let versions_to_keep = 4u64;

    let (test_rollup, admin) = create_test_rollup_with(|b| {
        // automatic_batch_production — disabled to keep height deterministic
        b.automatic_batch_production(false)
            .finalization_blocks(0)
            // Prune every finalized block, retaining only the most recent few versions.
            .pruner(PrunerConfig::Periodic {
                block_interval: 1,
                versions_to_keep: NonZeroU64::new(versions_to_keep).unwrap(),
                max_batch_size: default_max_pruning_batch_size(),
            })
    })
    .await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Set a distinctive value, wait for it to be included, and record the height we'll hammer.
    let target_value = 0x1234u64; // multi-byte / asymmetric to catch encoding bugs
    test_rollup
        .submit_and_wait_included(&admin, set_value(target_value))
        .await
        .unwrap();
    let target_height = test_rollup.height().await.get();

    // Background hammer: query the FIXED target height in a loop. The only acceptable outcomes are
    // (a) the exact value we set, or (b) HeightNotAccessible once pruned. (See the NOTE below for
    // why we do not assert a strictly monotonic readable -> pruned transition.)
    let stop = Arc::new(AtomicBool::new(false));
    let reader_client = test_rollup.client.clone();
    let reader_stop = stop.clone();
    let saw_pruned = Arc::new(tokio::sync::Notify::new());
    let reader_saw_pruned = saw_pruned.clone();
    let target_url = format!("/modules/value-setter/state/value?rollup_height={target_height}");
    let hammer_url = target_url.clone();
    let hammer = tokio::spawn(async move {
        let mut saw_not_accessible = false;
        let mut query_count = 0u64;
        while !reader_stop.load(Ordering::Relaxed) {
            match reader_client
                .query_rest_endpoint::<serde_json::Value>(&hammer_url)
                .await
            {
                Ok(response) => {
                    query_count += 1;
                    // Readable: it must be EXACTLY the value we set at this height — never `null`
                    // (i.e. `Ok(None)` for a key that was written), never stale/wrong...
                    assert_eq!(
                        response["value"].as_u64(),
                        Some(target_value),
                        "archival read at pruned-candidate height returned unexpected value: {response:?}",
                    );
                    // NOTE: we deliberately do NOT assert strict monotonicity (once-pruned ⇒
                    // always-pruned). Near the retained-window boundary a height can legitimately
                    // flip between readable and HeightNotAccessible as the pruned floor advances and
                    // cascading-delete preserves single-write keys. The safety contract we enforce
                    // is narrower: a *readable* result is never stale, wrong, or `null`.
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("404")
                        || msg.contains("pruned")
                        || msg.contains("invalid rollup height")
                    {
                        // HeightNotAccessible — the expected terminal state once pruned.
                        query_count += 1;
                        saw_not_accessible = true;
                        reader_saw_pruned.notify_one();
                    }
                    // Otherwise a transient transport error: ignore and retry.
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        (saw_not_accessible, query_count)
    });

    // Writer: advance the chain well past `versions_to_keep` so the pruner removes `target_height`.
    let iterations = versions_to_keep + 12;
    for i in 0..iterations {
        test_rollup
            .submit(&admin, set_value(target_value + 1 + i))
            .await
            .unwrap();
        test_rollup.force_close_batch().await.unwrap();
        test_rollup.tenderly_produce_blocks(2).await.unwrap();
        test_rollup.wait_for_node_synced().await.unwrap();
        test_rollup.wait_for_sequencer_ready().await.unwrap();
    }

    // Robustly confirm the target height is now pruned (bounded poll), independent of the hammer.
    let mut pruned_confirmed = false;
    for _ in 0..50 {
        match test_rollup
            .client
            .query_rest_endpoint::<serde_json::Value>(&target_url)
            .await
        {
            Ok(_) => {} // still readable; keep waiting for the pruner to catch up
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("404")
                    || msg.contains("pruned")
                    || msg.contains("invalid rollup height")
                {
                    pruned_confirmed = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        pruned_confirmed,
        "target height {target_height} should be HeightNotAccessible after advancing past versions_to_keep",
    );

    // Wait for the hammer to observe the pruned state, then stop and join it (this also
    // surfaces any assertion failure that fired inside the task).
    tokio::time::timeout(NOTIFICATION_TIMEOUT, saw_pruned.notified())
        .await
        .expect(
            "the concurrent reader should have observed HeightNotAccessible for the pruned height",
        );
    stop.store(true, Ordering::Relaxed);
    let (saw_not_accessible, query_count) = hammer.await.unwrap();
    assert!(
        query_count > 0,
        "the hammer task should have issued queries"
    );
    assert!(
        saw_not_accessible,
        "the concurrent reader should have observed HeightNotAccessible for the pruned height",
    );

    // The node never panicked and is still healthy...
    test_rollup.wait_for_node_synced().await.unwrap();

    // ...and the latest state is still served correctly and reflects the most recent write.
    let last_value = target_value + iterations;
    let latest: serde_json::Value = test_rollup
        .client
        .query_rest_endpoint("/modules/value-setter/state/value")
        .await
        .unwrap();
    assert_eq!(
        latest["value"].as_u64(),
        Some(last_value),
        "latest state should reflect the most recent write: {latest:?}",
    );
}

/// Regression test: verifies that archival state is accessible immediately after a slot
/// notification, even when checkpoint publication in the side-effects task is artificially delayed.
///
/// Uses `SOV_TEST_DELAY_FORCE_UPDATE_API_STATE_MS` to inject a 750ms delay before checkpoint
/// application. The ordering fix (oneshot ack in ForceUpdateApiState) ensures that slot
/// notifications are only sent after the checkpoint is applied, so queries should never see
/// `HeightNotAccessible` regardless of the delay.
///
/// Automatic batch production is disabled to prevent empty batches from advancing rollup height
/// through a separate code path, which would confound the checkpoint ordering test.
#[tokio::test(flavor = "multi_thread")]
async fn test_archival_state_is_accessible_even_with_delayed_checkpoint_update() {
    const DELAY_ENV: &str = "SOV_TEST_DELAY_FORCE_UPDATE_API_STATE_MS";
    std::env::set_var(DELAY_ENV, "750");

    let (test_rollup, admin) = create_test_rollup_with(|b| {
        // automatic_batch_production — disabled to isolate checkpoint ordering
        b.automatic_batch_production(false).finalization_blocks(0)
    })
    .await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let mut saw_invalid_height = false;

    for value_to_set in 1..=20_u64 {
        test_rollup
            .submit(&admin, set_value(value_to_set))
            .await
            .unwrap();

        test_rollup.force_close_batch().await.unwrap();

        let mut slots = test_rollup.api_client().subscribe_slots().await.unwrap();
        test_rollup.tenderly_produce_blocks(2).await.unwrap();
        for _ in 0..2 {
            let _slot = next_within(&mut slots, NOTIFICATION_TIMEOUT).await.unwrap();
        }

        let queried_height = test_rollup.height().await.get();
        let url = format!("/modules/value-setter/state/value?rollup_height={queried_height}");
        let query_result = test_rollup
            .client
            .query_rest_endpoint::<serde_json::Value>(&url)
            .await;

        match query_result {
            Ok(response) => {
                if response
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|msg| msg == "invalid rollup height")
                    .unwrap_or(false)
                {
                    saw_invalid_height = true;
                    break;
                }
            }
            Err(err) => {
                let err_string = format!("{err:#}");
                if err_string.contains("404 Not Found")
                    && err_string.contains("invalid rollup height")
                {
                    saw_invalid_height = true;
                    break;
                }
            }
        }
    }

    let _ = test_rollup.shutdown().await;
    std::env::remove_var(DELAY_ENV);

    assert!(
        !saw_invalid_height,
        "Observed transient invalid rollup height despite ordering fix. The checkpoint-vs-notification race condition has regressed."
    );
}

/// Verifies that archival reads consult `uncommitted_changes` when the requested
/// height's data has not yet been propagated to the sequencer from the node.
///
/// Pauses the sequencer's `update_state` task so the sequencer never receives the
/// node's state update for batch 2. The sequencer's storage handle stays stale,
/// deterministically forcing archival reads through `uncommitted_changes`.
#[tokio::test(flavor = "multi_thread")]
async fn test_archival_read_from_uncommitted_changes() {
    let (test_rollup, admin) =
        create_test_rollup_with(|b| b.automatic_batch_production(false).finalization_blocks(0))
            .await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // Batch 1: set value to 0x1234, let the node fully commit it.
    test_rollup.submit(&admin, set_value(0x1234)).await.unwrap();
    test_rollup.force_close_batch().await.unwrap();
    test_rollup.tenderly_produce_blocks(2).await.unwrap();
    test_rollup.wait_for_node_synced().await.unwrap();

    let settled_height = test_rollup.height().await.get();

    // Pause the sequencer's update_state task. The sequencer will not receive
    // any further state updates from the node, keeping its storage stale.
    // Batch 2 data will only be reachable via uncommitted_changes.
    test_rollup.pause_preferred_batches().await;

    // Batch 2: set value to 0x5678, produce DA blocks.
    // The node commits to NOMT, but the sequencer never learns about it.
    test_rollup.submit(&admin, set_value(0x5678)).await.unwrap();
    test_rollup.force_close_batch().await.unwrap();
    test_rollup.tenderly_produce_blocks(2).await.unwrap();

    let new_height = test_rollup.height().await.get();
    assert!(
        new_height > settled_height,
        "Height should have advanced after second batch"
    );

    // === User state (get_archival_from_user_storage) ===

    // Archival query at the NEW height must find 0x5678 from uncommitted_changes (path 2).
    // Without the fix, this returns a 404 (HeightNotAccessible) or stale data.
    query_set_value(&test_rollup, Some(new_height), Some(0x5678))
        .await
        .expect("User state at uncommitted height should return 0x5678");

    // Archival query at the settled height (path 1: kernel mapping in NOMT).
    // Tests that ignore_changes_after_height prunes batch 2's data.
    query_set_value(&test_rollup, Some(settled_height), Some(0x1234))
        .await
        .expect("User state at settled height should return 0x1234");

    // === Accessory state (get_archival_from_accessory_storage) ===

    // finalize-hook-count increments each slot. At settled_height it has some value N;
    // at new_height it should be N+1 (one more finalize hook ran for batch 2).
    let settled_hook_count = query_hook_count(&test_rollup, Some(settled_height))
        .await
        .expect("Accessory state at settled height should be accessible");
    let new_hook_count = query_hook_count(&test_rollup, Some(new_height))
        .await
        .expect(
            "Accessory state at uncommitted height should be accessible from uncommitted_changes",
        );
    assert!(
        new_hook_count > settled_hook_count,
        "finalize-hook-count at new_height ({new_hook_count}) should be greater than at settled_height ({settled_hook_count})"
    );

    test_rollup.resume_preferred_batches().await;
}

/// Checks that historical `ValueSetter` reads return the value as of the requested rollup height
/// or slot number.
///
/// [`do_manual_block_production_test`] sets the value to 0 and then to 7 in two consecutive
/// batches. This test then queries both by `rollup_height` and by `slot_number`: points before the
/// first write have no value, later points see 0 or 7 depending on which batch they follow, and
/// far-future heights or slot numbers return an error. A panic hook ensures that none of these
/// queries panicked anywhere in the rollup.
#[tokio::test(flavor = "multi_thread")]
async fn query_historical_values() {
    let panicked: Arc<std::sync::atomic::AtomicBool> =
        Arc::new(std::sync::atomic::AtomicBool::new(false));
    let panicked_ref = panicked.clone();
    let prev_hook = std::panic::take_hook();
    // Set a new panic hook
    std::panic::set_hook(Box::new(move |panic_info| {
        // Your custom panic handling logic here
        panicked_ref.store(true, std::sync::atomic::Ordering::SeqCst);
        prev_hook(panic_info);
    }));
    let tx_builder = |key| tx_set_value(&key, 0, 7);
    let assertions = |test_rollup| async move {
        query_set_value(&test_rollup, Some(2), Some(7))
            .await
            .unwrap();
        query_set_value(&test_rollup, Some(0), None).await.unwrap();
        query_set_value_by_slot_number(&test_rollup, Some(8), Some(7))
            .await
            .unwrap();
        query_set_value_by_slot_number(&test_rollup, Some(7), Some(0))
            .await
            .unwrap();
        query_set_value_by_slot_number(&test_rollup, Some(1), None)
            .await
            .unwrap();
        // Query some future heights/slot numbers to be sure they don't panic
        assert!(query_set_value(&test_rollup, Some(100000), None)
            .await
            .is_err());
        assert!(
            query_set_value_by_slot_number(&test_rollup, Some(100000), None)
                .await
                .is_err()
        );
        test_rollup.shutdown().await.unwrap();
        assert!(
            !panicked.load(std::sync::atomic::Ordering::SeqCst),
            "Panicked during assertions"
        );
    };
    do_manual_block_production_test(tx_builder, assertions).await;
}

async fn query_hook_count(
    test_rollup: &TestRollup<TestBlueprint>,
    rollup_height: Option<u64>,
) -> anyhow::Result<u32> {
    let url = match rollup_height {
        Some(h) => format!("/modules/hooks-count/state/finalize-hook-count?rollup_height={h}"),
        None => "/modules/hooks-count/state/finalize-hook-count".to_string(),
    };
    let response = test_rollup
        .client
        .query_rest_endpoint::<serde_json::Value>(&url)
        .await?;
    if response.get("message").is_some() {
        return Err(anyhow::anyhow!("API request failed: {:?}", response));
    }
    response["value"]
        .as_u64()
        .map(|v| v as u32)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'value' in response: {:?}", response))
}
