//! Tests for shutdown/restart cases.
use std::collections::HashSet;
use std::env;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context;
use futures::StreamExt;
use rand::Rng;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::OperatingMode;
use sov_modules_rollup_blueprint::logging::default_rust_log_value;
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_sequencer::SequencerKindConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::logging::LogCollector;
use sov_test_utils::test_rollup::{RollupBuilder, TestRollup};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{registry, EnvFilter, Layer};

use crate::test_helpers::test_genesis_source;

const ROLLUP_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const ROLLUP_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const FULL_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

fn initialize_logging_for_restart(collector: LogCollector, with_stdout: bool) {
    let subscriber = registry().with(collector);
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        tracing_panic::panic_hook(panic_info);
        prev_hook(panic_info);
    }));
    if with_stdout {
        let env_filter =
            env::var("RUST_LOG").unwrap_or_else(|_| default_rust_log_value().to_string());

        let get_env_filter = || EnvFilter::from_str(&env_filter).unwrap();
        let layer = tracing_subscriber::fmt::layer()
            .with_filter(get_env_filter())
            .boxed();

        subscriber.with(layer).init();
    } else {
        subscriber.init();
    }
}

/// Starts a TestNode, lets it run for some time and then shuts it down.
/// Repeats that several times.
/// Rollup and MockDa data are preserved between restarts.
async fn start_stop_empty(
    operation_mode: OperatingMode,
    finalization_blocks: u32,
    rollup_prover_config: RollupProverConfig<Risc0>,
) -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    let subscriber = registry().with(collector.clone());
    subscriber.init();

    let rollup_storage_dir = Arc::new(tempfile::tempdir()?);
    let restarts = 50;
    let mut rng = rand::thread_rng();

    let sleep_durations: Vec<std::time::Duration> = (0..restarts)
        .map(|_| std::time::Duration::from_millis(rng.gen_range(80..=300)))
        .collect();

    for sleep_duration in sleep_durations {
        let test_rollup = tokio::time::timeout(
            ROLLUP_START_TIMEOUT,
            RollupBuilder::<MockDemoRollup<Native>>::new(
                test_genesis_source(operation_mode),
                TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
                finalization_blocks,
            )
            .with_zkvm_host_args(mock_da_risc0_host_args())
            .set_config(|c| {
                c.max_concurrent_blobs = 65536;
                c.storage = rollup_storage_dir.clone();
                c.rollup_prover_config = Some(rollup_prover_config.clone());
                if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
                    sequencer_conf.disable_state_root_consistency_checks = true;
                }
                c.aggregated_proof_block_jump = 10;
            })
            .set_persistent_da()
            .start(),
        )
        .await
        .context("Starting rollup failed")??;

        // Let rollup run for some time
        tokio::time::sleep(sleep_duration).await;

        let TestRollup {
            shutdown_sender,
            rollup_task,
            da_service,
            ..
        } = test_rollup;

        drop(da_service);
        tracing::info!("Triggering shutdown....");
        shutdown_sender.send(())?;
        tokio::time::timeout(ROLLUP_SHUTDOWN_TIMEOUT, rollup_task)
            .await
            .context("Joining rollup task failed")???;

        // // By design, child tasks don't always report back to their parents when they finish shutting down. This is fine
        // // during normal operation, but it means that we can't "await" until every spawned task is shutdown for this test. That makes
        // // the test flaky, since we sometimes try to restart the rollup before we finish shutting it down, causing rocksdb locks to trigger.
        // // A small sleep prevents this.
        // tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let known = [
        // https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1878:
        (
            Level::ERROR,
            "Invalid proof outcome".to_string(),
        ),
        (
            Level::WARN,
            "Received error updating target height, stopping background task".to_string()
        ),
        // The node gets out of sync during the restart
        (
            Level::WARN,
            "The sequencer must pause because the node has lagged behind the DA blockchain. This might lead to a brief downtime for users.".to_string()
        ),
        (Level::WARN, "Skipping pruning of sequence number because it's already been pruned".to_string()),
    ];

    let mut recorded_errors_warnings =
        HashSet::<(Level, String)>::from_iter(collector.records().iter().cloned());
    recorded_errors_warnings.retain(|e| !known.contains(e));
    // We could've checked `.is_empty`, but in case of failure, we will see errors immediately.
    assert_eq!(HashSet::<(Level, String)>::new(), recorded_errors_warnings);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_zk_instant_finality() -> anyhow::Result<()> {
    start_stop_empty(OperatingMode::Zk, 0, RollupProverConfig::Skip).await?;
    // if can_execute_zk_guest() {
    //     start_stop_empty(OperatingMode::Zk, 0, RollupProverConfig::Execute).await?;
    // }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_zk_non_instant_finality() -> anyhow::Result<()> {
    start_stop_empty(OperatingMode::Zk, 3, RollupProverConfig::Skip).await?;
    // if can_execute_zk_guest() {
    //     start_stop_empty(OperatingMode::Zk, 3, RollupProverConfig::Execute).await?;
    // }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_optimistic_instant_finality() -> anyhow::Result<()> {
    start_stop_empty(OperatingMode::Optimistic, 0, RollupProverConfig::Skip).await?;
    // if can_execute_zk_guest() {
    //     start_stop_empty(OperatingMode::Optimistic, 0, RollupProverConfig::Execute).await?;
    // }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_optimistic_non_instant_finality() -> anyhow::Result<()> {
    start_stop_empty(OperatingMode::Optimistic, 3, RollupProverConfig::Skip).await?;
    // if can_execute_zk_guest() {
    //     start_stop_empty(OperatingMode::Optimistic, 3, RollupProverConfig::Execute).await?;
    // }
    Ok(())
}

// fn can_execute_zk_guest() -> bool {
//     let skip_guest_build = std::env::var("SKIP_GUEST_BUILD").unwrap_or_default();
//     matches!(skip_guest_build.to_lowercase().as_str(), "" | "0" | "false")
// }

#[tokio::test(flavor = "multi_thread")]
async fn test_start_prover_manual() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false); // Enable stdout logging. Set to false to disable.

    let rollup_storage_dir = Arc::new(tempfile::tempdir()?);
    let finalization_blocks = 0;

    let first_chunk = 6;
    let second_chunk = 4;
    let jump_size = first_chunk + second_chunk;

    let rollup_builder = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Zk),
        BlockProducingConfig::Periodic {
            block_time_ms: 1000,
        },
        finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.storage = rollup_storage_dir.clone();
        c.rollup_prover_config = Some(RollupProverConfig::Skip);
        // Since we have the prover enabled, we need to disable state root consistency checks.
        if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
            sequencer_conf.disable_state_root_consistency_checks = true;
        }
        c.aggregated_proof_block_jump = jump_size;
    });

    let mock_da_dir = &rollup_storage_dir;

    {
        let mut storable_mock_da_layer =
            StorableMockDaLayer::new_in_path(mock_da_dir.path(), 0).await?;
        for _ in 0..first_chunk {
            storable_mock_da_layer.produce_block().await?;
        }
    }

    {
        let test_rollup = rollup_builder.clone().start().await?;
        let mut slot_subscription = test_rollup.client.client.subscribe_slots().await?;

        let TestRollup {
            shutdown_sender,
            rollup_task,
            ..
        } = test_rollup;

        let rollup_height = slot_subscription
            .next()
            .await
            .transpose()?
            .map(|slot| slot.number)
            .unwrap_or_default();

        if rollup_height < first_chunk as u64 {
            let till = first_chunk - rollup_height as usize;
            for _ in 0..till {
                let _ = slot_subscription.next().await.unwrap();
            }
        }
        drop(slot_subscription);

        shutdown_sender.send(())?;
        let _ = rollup_task.await?;
    }

    let _head_before_restart = {
        let mut storable_mock_da_layer =
            StorableMockDaLayer::new_in_path(mock_da_dir.path(), 0).await?;
        for _ in 0..=second_chunk {
            storable_mock_da_layer.produce_block().await?;
        }
        storable_mock_da_layer
            .get_head_block_header()
            .await?
            .height()
    };

    {
        let test_rollup = rollup_builder.start().await?;

        let mut slot_subscription = test_rollup.client.client.subscribe_slots().await?;

        let TestRollup {
            shutdown_sender,
            rollup_task,
            ..
        } = test_rollup;

        let rollup_height = slot_subscription
            .next()
            .await
            .transpose()?
            .map(|slot| slot.number)
            .unwrap_or_default();
        if rollup_height < second_chunk as u64 {
            let till = second_chunk - rollup_height as usize;
            for _ in 0..till {
                let _ = slot_subscription.next().await.unwrap();
            }
        }
        drop(slot_subscription);

        // FIXME(@theochap, `<https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1907>`): this assertion is broken because of a race condition in the preferred sequencer.
        // // We give rollup 1 second to produce mock proof.
        // for _ in 0..10 {
        //     let head_after_restart = da_service.get_head_block_header().await?;
        //     if head_after_restart.height() > head_before_restart {
        //         break;
        //     }
        //     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // }

        // let head_after_restart = da_service.get_head_block_header().await?;
        // assert_eq!(
        //     head_after_restart.height(),
        //     head_before_restart + 1,
        //     "Prover hasn't posted proof"
        // );

        shutdown_sender.send(())?;
        let _ = rollup_task.await?;
    }

    let mut recorded_errors_warnings =
        HashSet::<(Level, String)>::from_iter(collector.records().iter().cloned());
    let known = [
        // Error because of ledger subscription
        (Level::WARN, "WebSocket error".to_string()),
        (
            Level::WARN,
            "Received error updating target height, stopping background task".to_string(),
        ),
    ];
    recorded_errors_warnings.retain(|e| !known.contains(e));
    // We could've checked `.is_empty`, but in case of failure, we will see errors immediately.
    assert_eq!(HashSet::<(Level, String)>::new(), recorded_errors_warnings);

    Ok(())
}

// Test setup is sneaky and might be redundant if prover takes its own database.
// Basically, ST info is saved to the storage manager in the same loop iteration,
// but notified height is written in the next iteration.
// In each loop we submit several blocks, enough to keep the runner busy.
// We cannot be 100% sure that extra ST info will be written each time,
// That's why we do more restarts comparing to channel size.
// The downside of this test is that it won't fail if there's no bug,
// But it might succeed if there's a bug.
async fn check_with_increasing_stf_infos(
    operating_mode: OperatingMode,
    finalization_blocks: u32,
    aggregated_proof_jump: usize,
    max_channel_size: u64,
    max_infos_in_db: u64,
    restarts: usize,
    blocks_per_start: usize,
) -> anyhow::Result<()> {
    // Checks startup process isn't dead locked.
    let rollup_storage_dir = Arc::new(tempfile::tempdir()?);

    let rollup_builder = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(operating_mode),
        BlockProducingConfig::Manual,
        finalization_blocks,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.storage = rollup_storage_dir.clone();
        c.rollup_prover_config = Some(RollupProverConfig::Skip);
        c.aggregated_proof_block_jump = aggregated_proof_jump;
        c.max_channel_size = max_channel_size;
        c.max_infos_in_db = max_infos_in_db;
    });

    let mut last_processed_slot_number = 0;
    for idx in 0..restarts {
        let test_rollup =
            tokio::time::timeout(ROLLUP_START_TIMEOUT, rollup_builder.clone().start())
                .await
                .with_context(|| format!("start n={idx} of the rollup failed"))??;

        let TestRollup {
            shutdown_sender,
            rollup_task,
            da_service,
            client: sov_cli::NodeClient {
                client: api_client, ..
            },
            ..
        } = test_rollup;
        let da_service_ref = da_service.clone();
        let mut slot_subscription = api_client.subscribe_slots().await?;

        // Produce enough blocks to fill the channel and accommodate slot processing time.
        for _ in 0..blocks_per_start {
            da_service_ref.produce_block_now().await?;
        }

        let slot =
            tokio::time::timeout(std::time::Duration::from_secs(10), slot_subscription.next())
                .await
                .context("waiting for next slot is failed")?
                .transpose()?
                .unwrap();
        assert!(
            slot.number > last_processed_slot_number,
            "Received notification for slot n={} is lower than last seen: {}",
            slot.number,
            last_processed_slot_number
        );
        last_processed_slot_number = slot.number;

        drop(slot_subscription);
        drop(da_service);
        shutdown_sender.send(())?;
        tokio::time::timeout(ROLLUP_SHUTDOWN_TIMEOUT, rollup_task)
            .await
            .context("Joining rollup task failed")???;
    }

    Ok(())
}

async fn try_to_clog_channel_instant_finality(operating_mode: OperatingMode) -> anyhow::Result<()> {
    let max_channel_size = 5;
    // We assume that each restart we produce 1 extra STF info with 10% probability
    let restarts = 50;
    // Submission to MockDa is faster than processing single slot
    // and with more data in StateDb single slot processing time should slightly degrade
    let blocks_per_start = 30;

    // Never produce aggregated proof
    let aggregated_proof_jump: usize = 200;
    let max_infos_in_db = 500;

    tokio::time::timeout(
        FULL_TEST_TIMEOUT,
        check_with_increasing_stf_infos(
            operating_mode,
            1,
            aggregated_proof_jump,
            max_channel_size,
            max_infos_in_db,
            restarts,
            blocks_per_start,
        ),
    )
    .await?
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
// Flaky because of an existing problem: "Error: IO error: lock hold by current process,"
async fn flaky_test_increasing_stf_infos_zk_instant_finality() -> anyhow::Result<()> {
    try_to_clog_channel_instant_finality(OperatingMode::Zk).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
// Flaky because of an existing problem: "Error: IO error: lock hold by current process,"
async fn flaky_test_increasing_stf_infos_optimistic_instant_finality() -> anyhow::Result<()> {
    try_to_clog_channel_instant_finality(OperatingMode::Optimistic).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Disabled while https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1924"]
async fn flaky_try_to_clog_db_zk_instant_finality() -> anyhow::Result<()> {
    let max_channel_size = 100;
    let max_infos_in_db = 5;
    // We assume that each restart we produce 1 extra STF info with 10% probability
    let restarts = 50;
    // Submission to MockDa is faster than processing a single slot,
    // and with more data in StateDb single slot processing time should slightly degrade
    let blocks_per_start = 30;

    // Never produce aggregated proof
    let aggregated_proof_jump: usize = 200;

    check_with_increasing_stf_infos(
        OperatingMode::Zk,
        1,
        aggregated_proof_jump,
        max_channel_size,
        max_infos_in_db,
        restarts,
        blocks_per_start,
    )
    .await
}
