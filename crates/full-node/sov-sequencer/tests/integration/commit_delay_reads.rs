//! Reads through the REST API while every storage commit sleeps at one fault-injection point.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sov_db::test_utils::{CommitFaultInjectionLocation, SLEEP_ON_COMMIT_MS_ENV_NAME};
use sov_mock_da::BlockProducingConfig;
use sov_test_utils::{TestSpec, TEST_BLOB_PROCESSING_TIMEOUT, TEST_MAX_BATCH_SIZE};
use tokio::time::sleep;
use tokio_stream::StreamExt;

use crate::preferred_end_to_end::{create_genesis_params, tx_set_value, TestRuntime};
use crate::utils::{new_test_rollup, tempdir_inside_codebase_dir, MAX_BATCH_EXECUTION_TIME_MILLIS};

/// Latest and archival reads through the REST API stay correct while every storage commit sleeps
/// at `location`. The sleep changes which store finishes first in the parallel commit and how long
/// the flat DB waits before its live write, so readers race with every phase of the commit.
///
/// A background reader checks that the latest value never goes backwards and that every height the
/// writer has already observed returns exactly the value set at that height.
async fn reads_stay_consistent_with_commit_delay_at(location: CommitFaultInjectionLocation) {
    const COMMIT_DELAY_MS: u64 = 50;
    const ITERATIONS: u64 = 10;
    sov_test_utils::initialize_logging();
    location.set_sleep_env(COMMIT_DELAY_MS);

    let (genesis_params, admin) = create_genesis_params();
    let dir = tempdir_inside_codebase_dir();
    let test_rollup = new_test_rollup::<TestRuntime<TestSpec>>(
        dir.clone(),
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
        0,
        false, // automatic_batch_production — disabled so each iteration adds exactly one rollup height
        TEST_MAX_BATCH_SIZE,
        BlockProducingConfig::Manual,
        None,
        TEST_BLOB_PROCESSING_TIMEOUT,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
        None,
        0,
    )
    .await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    // (rollup height, value set at that height), published once the height is synced.
    let written: Arc<std::sync::RwLock<Vec<(u64, u64)>>> = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let client = test_rollup.client.clone();
        let written = written.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut last_latest = None;
            let mut rounds = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let latest = client
                    .query_rest_endpoint::<serde_json::Value>("/modules/value-setter/state/value")
                    .await
                    .expect("Latest value query failed");
                let latest = latest["value"].as_u64();
                assert!(
                    latest >= last_latest,
                    "Latest value went backwards from {last_latest:?} to {latest:?}"
                );
                last_latest = latest;

                let heights = written.read().unwrap().clone();
                for (height, expected) in heights {
                    let url = format!("/modules/value-setter/state/value?rollup_height={height}");
                    let response = client
                        .query_rest_endpoint::<serde_json::Value>(&url)
                        .await
                        .unwrap_or_else(|e| {
                            panic!("Archival query at height {height} failed: {e}")
                        });
                    assert_eq!(
                        response["value"].as_u64(),
                        Some(expected),
                        "Archival read at height {height} returned a wrong value: {response:?}"
                    );
                }
                rounds += 1;
                sleep(Duration::from_millis(5)).await;
            }
            rounds
        })
    };

    // A height becomes readable once the next batch is closed, so each height is published one
    // iteration after it was written.
    let mut pending: Option<(u64, u64)> = None;
    for generation in 0..ITERATIONS {
        // Multi-byte, asymmetric values.
        let value = 0x1234 + generation;
        let tx = tx_set_value(&admin.private_key, generation, value);
        test_rollup
            .api_client()
            .send_raw_tx_to_sequencer(&tx)
            .await
            .unwrap();
        test_rollup.force_close_batch().await.unwrap();

        let mut slots = test_rollup.api_client().subscribe_slots().await.unwrap();
        test_rollup.tenderly_produce_blocks(2).await.unwrap();
        test_rollup.wait_for_node_synced().await.unwrap();
        test_rollup.wait_for_sequencer_ready().await.unwrap();
        // Slot notifications mean the API state got all updates.
        for _ in 0..2 {
            let _slot = slots.next().await.unwrap().unwrap();
        }
        if let Some(previous) = pending.take() {
            written.write().unwrap().push(previous);
        }
        pending = Some((test_rollup.height().await.get(), value));
    }

    stop.store(true, Ordering::Relaxed);
    let rounds = reader.await.unwrap();
    assert!(rounds > 0, "The reader must have run during the commits");

    let _ = test_rollup.shutdown().await;
    std::env::remove_var(SLEEP_ON_COMMIT_MS_ENV_NAME);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reads_stay_consistent_with_delay_before_kernel_nomt_commit() {
    reads_stay_consistent_with_commit_delay_at(
        CommitFaultInjectionLocation::BeforeCommittingKernelNomt,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reads_stay_consistent_with_delay_before_user_nomt_commit() {
    reads_stay_consistent_with_commit_delay_at(
        CommitFaultInjectionLocation::BeforeCommittingUserNomt,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reads_stay_consistent_with_delay_before_ledger_commit() {
    reads_stay_consistent_with_commit_delay_at(
        CommitFaultInjectionLocation::BeforeCommittingLedger,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reads_stay_consistent_with_delay_before_accessory_commit() {
    reads_stay_consistent_with_commit_delay_at(
        CommitFaultInjectionLocation::BeforeCommittingAccessory,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reads_stay_consistent_with_delay_before_archival_commit() {
    reads_stay_consistent_with_commit_delay_at(
        CommitFaultInjectionLocation::BeforeCommittingArchival,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reads_stay_consistent_with_delay_before_live_commit() {
    reads_stay_consistent_with_commit_delay_at(CommitFaultInjectionLocation::BeforeCommittingLive)
        .await;
}
