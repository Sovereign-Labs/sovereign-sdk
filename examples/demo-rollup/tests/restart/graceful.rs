//! Tests for shutdown/restart cases.
use std::collections::HashSet;
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::test_helpers::{
    build_transfer_token_tx_with_generation, test_genesis_source, DemoRollupSpec,
};
use anyhow::Context;
use futures::StreamExt;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use sov_bank::config_gas_token_id;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::{BlockProducingConfig, MockDaConfig};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CryptoSpec, OperatingMode, PrivateKey, PublicKey, Spec};
use sov_modules_rollup_blueprint::logging::default_rust_log_value;
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_sequencer::SequencerKindConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::generate_operator_runtime_with_kernel;
use sov_test_utils::logging::LogCollector;
use sov_test_utils::test_rollup::{read_private_key, RollupBuilder, StoragePath, TestRollup};
use sov_test_utils::{TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS, TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING};
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{registry, EnvFilter, Layer};

generate_operator_runtime_with_kernel!(
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    TestRuntime <=
);

const ROLLUP_START_TIMEOUT: Duration = Duration::from_secs(10);
const ROLLUP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const FULL_TEST_TIMEOUT: Duration = Duration::from_secs(300);
const UNDER_LOAD_READY_TIMEOUT: Duration = Duration::from_secs(3);
const MIN_SLEEP_MS: i64 = 10;
const JITTER_MS: i64 = 15;
const TX_SEND_INTERVAL_MS: u64 = 50;
const TX_SENDER_DRAIN_BEFORE_SHUTDOWN_MS: u64 = 250;

#[derive(Clone, Copy, Debug)]
enum SleepKind {
    Exact,
    Jittered,
}

#[derive(Clone, Copy, Debug)]
struct SleepScheduleEntry {
    duration: Duration,
    kind: SleepKind,
}

/// Builds predefined sleep durations spanning the full range relative to block time.
///
/// Categories cover: early startup, just before/at/after block production,
/// and multi-block territory — ensuring restarts happen at every interesting phase.
fn predefined_sleep_durations(block_time_ms: u64) -> Vec<Duration> {
    let bt = block_time_ms as i64;
    [
        30,           // barely started — initialization phase
        100,          // early startup — components initializing
        300,          // mid-startup — close to first block
        bt - 30,      // just before block production (420ms)
        bt,           // exactly at block production time (450ms)
        bt + 30,      // just after block production (480ms)
        500,          // first block likely done
        800,          // several blocks worth
        bt * 2,       // two full block cycles (900ms)
        bt * 3 + 100, // well into multi-block territory (1450ms)
    ]
    .into_iter()
    .map(|ms| Duration::from_millis(ms.max(10) as u64))
    .collect()
}

fn build_sleep_schedule(
    base_durations: &[Duration],
    exact_repeats: usize,
    jitter_repeats: usize,
    rng: &mut StdRng,
) -> Vec<SleepScheduleEntry> {
    let mut schedule = Vec::with_capacity(base_durations.len() * (exact_repeats + jitter_repeats));
    for base in base_durations {
        for _ in 0..exact_repeats {
            schedule.push(SleepScheduleEntry {
                duration: *base,
                kind: SleepKind::Exact,
            });
        }
        for _ in 0..jitter_repeats {
            let jitter_ms = rng.gen_range(-JITTER_MS..=JITTER_MS);
            let ms = base.as_millis() as i64 + jitter_ms;
            schedule.push(SleepScheduleEntry {
                duration: Duration::from_millis(ms.max(MIN_SLEEP_MS) as u64),
                kind: SleepKind::Jittered,
            });
        }
    }
    schedule.shuffle(rng);
    schedule
}

fn known_restart_warnings() -> [(Level, String); 9] {
    [
        // https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1878:
        (
            Level::ERROR,
            "Invalid proof outcome".to_string(),
        ),
        (
            Level::WARN,
            "Received error updating target height, stopping background task".to_string(),
        ),
        // The node gets out of sync during the restart
        (
            Level::WARN,
            "The sequencer must pause because the node has lagged behind the DA blockchain. This might lead to a brief downtime for users.".to_string(),
        ),
        (
            Level::WARN,
            "Skipping pruning of sequence number because it's already been pruned".to_string(),
        ),
        (
            Level::WARN,
            "The node is unsynced and doesn't know it. This probably means that you wiped the node DB and are resyncing.".to_string(),
        ),
        (
            Level::WARN,
            "Metrics have been initialized outside of the rollup blueprint, some measurements can be lost on shutdown".to_string(),
        ),
        // Expected during catch-up/restart races when replayed txs cannot be applied.
        (
            Level::WARN,
            "Cache warm up task: Transaction could not be applied on the executor.".to_string(),
        ),
        // Emitted by tower-http TraceLayer for non-success responses (e.g. /sequencer/ready probes).
        (
            Level::ERROR,
            "response failed".to_string(),
        ),
        // Transactions can race with readiness transitions around restart boundaries.
        (
            Level::ERROR,
            "Error accepting transaction".to_string(),
        ),
    ]
}

fn assert_only_known_logs_since(collector: &LogCollector, start_idx: usize) {
    let known = known_restart_warnings();
    let mut recorded_errors_warnings =
        HashSet::<(Level, String)>::from_iter(collector.records().into_iter().skip(start_idx));
    recorded_errors_warnings.retain(|e| !known.contains(e));
    // We could've checked `.is_empty`, but in case of failure, we will see errors immediately.
    assert_eq!(HashSet::<(Level, String)>::new(), recorded_errors_warnings);
}

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
///
/// Uses predefined sleep durations relative to block time and a seeded RNG
/// for reproducible shuffling. Log the seed so failures can be reproduced.
async fn start_stop_empty(
    operation_mode: OperatingMode,
    finalization_blocks: u32,
    rollup_prover_config: RollupProverConfig<Risc0>,
    seed: u64,
    collector: &LogCollector,
) -> anyhow::Result<()> {
    let log_start_idx = collector.records().len();
    tracing::info!(seed, "Starting start_stop_empty with seed");

    let rollup_storage_dir = Arc::new(tempfile::tempdir()?);
    let mut rng = StdRng::seed_from_u64(seed);
    let base_durations = predefined_sleep_durations(TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS);

    // 10 categories * (3 exact + 2 jittered) = 50 restarts
    let sleep_schedule = build_sleep_schedule(&base_durations, 3, 2, &mut rng);

    for (i, sleep_entry) in sleep_schedule.iter().enumerate() {
        tracing::info!(
            restart = i,
            sleep_kind = ?sleep_entry.kind,
            sleep_ms = sleep_entry.duration.as_millis() as u64,
            "Restart iteration"
        );
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
                c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
                c.rollup_prover_config = Some(rollup_prover_config.clone());
                if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
                    sequencer_conf.disable_state_root_consistency_checks = true;
                    sequencer_conf.ideal_lag_behind_finalized_slot = 3;
                }
                c.aggregated_proof_block_jump = 10;
            })
            .set_persistent_da()
            .start(),
        )
        .await
        .context("Starting rollup failed")??;

        // Let rollup run for some time
        tokio::time::sleep(sleep_entry.duration).await;

        tracing::info!("Triggering shutdown....");
        tokio::time::timeout(ROLLUP_SHUTDOWN_TIMEOUT, test_rollup.shutdown()).await??;
    }

    assert_only_known_logs_since(collector, log_start_idx);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_zk_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_empty(
            OperatingMode::Zk,
            0,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_zk_non_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_empty(
            OperatingMode::Zk,
            3,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_optimistic_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_empty(
            OperatingMode::Optimistic,
            0,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_optimistic_non_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_empty(
            OperatingMode::Optimistic,
            3,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

/// Like `start_stop_empty`, but sends transactions in the background while
/// the rollup runs, testing restart behavior under load.
///
/// Uses fewer restarts (2x multiplier = 20) since each iteration does more work.
async fn start_stop_under_load(
    operation_mode: OperatingMode,
    finalization_blocks: u32,
    rollup_prover_config: RollupProverConfig<Risc0>,
    seed: u64,
    collector: &LogCollector,
) -> anyhow::Result<()> {
    let log_start_idx = collector.records().len();
    tracing::info!(seed, "Starting start_stop_under_load with seed");

    let rollup_storage_dir = Arc::new(tempfile::tempdir()?);
    let mut rng = StdRng::seed_from_u64(seed);
    let base_durations = predefined_sleep_durations(TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS);

    // 10 categories * (1 exact + 1 jittered) = 20 restarts
    let sleep_schedule = build_sleep_schedule(&base_durations, 1, 1, &mut rng);

    let key_and_address = read_private_key::<DemoRollupSpec>("tx_signer_private_key.json");
    let receiver_addr: <DemoRollupSpec as Spec>::Address = {
        let pk = <<DemoRollupSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
        pk.pub_key().credential_id().into()
    };
    let token_id = config_gas_token_id();

    let mut generation: u64 = 0;

    for (i, sleep_entry) in sleep_schedule.iter().enumerate() {
        tracing::info!(
            restart = i,
            sleep_kind = ?sleep_entry.kind,
            sleep_ms = sleep_entry.duration.as_millis() as u64,
            "Under-load restart iteration"
        );
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
                c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
                c.rollup_prover_config = Some(rollup_prover_config.clone());
                if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
                    sequencer_conf.disable_state_root_consistency_checks = true;
                    sequencer_conf.ideal_lag_behind_finalized_slot = 3;
                }
                c.aggregated_proof_block_jump = 10;
            })
            .set_persistent_da()
            .start(),
        )
        .await
        .with_context(|| format!("Starting rollup failed: restart={i} seed={seed}"))??;

        // Submit transactions only while the sequencer is known-ready.
        let sequencer_ready = tokio::time::timeout(UNDER_LOAD_READY_TIMEOUT, async {
            loop {
                if test_rollup.is_sequencer_ready().await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .is_ok();

        if !sequencer_ready {
            tracing::info!(
                restart = i,
                seed,
                "Skipping tx phase because sequencer did not become ready in time"
            );
        }

        let tx_sender_state = if sequencer_ready {
            let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
            let api_client = test_rollup.client.client.clone();
            let sender_key = key_and_address.private_key.clone();
            let sender_receiver = receiver_addr;
            let start_generation = generation;
            let tx_sender = tokio::spawn(async move {
                let mut count = 0u64;
                loop {
                    let tx = build_transfer_token_tx_with_generation::<DemoRollupSpec>(
                        &sender_key,
                        token_id,
                        sender_receiver,
                        100,
                        start_generation.saturating_add(count),
                    );

                    // Race cancellation against the send so we respond promptly to shutdown
                    tokio::select! {
                        _ = &mut cancel_rx => break,
                        _ = api_client.send_tx_to_sequencer(&tx) => {}
                    }
                    count += 1;

                    // Pace the sends, also checking for cancellation
                    tokio::select! {
                        _ = &mut cancel_rx => break,
                        _ = tokio::time::sleep(Duration::from_millis(TX_SEND_INTERVAL_MS)) => {}
                    }
                }
                count
            });
            Some((cancel_tx, tx_sender))
        } else {
            None
        };

        // Let rollup run under load for some time
        tokio::time::sleep(sleep_entry.duration).await;

        let mut txs_sent = 0u64;
        if let Some((cancel_tx, tx_sender)) = tx_sender_state {
            // Stop tx sender before shutdown so submissions happen only while ready.
            let _ = cancel_tx.send(());
            txs_sent = tokio::time::timeout(ROLLUP_SHUTDOWN_TIMEOUT, tx_sender)
                .await
                .with_context(|| format!("join_tx_sender timed out: restart={i} seed={seed}"))?
                .with_context(|| format!("tx sender task panicked: restart={i} seed={seed}"))?;
            generation = generation.saturating_add(txs_sent);

            // Give in-flight request futures a short window to settle before shutdown.
            tokio::time::sleep(Duration::from_millis(TX_SENDER_DRAIN_BEFORE_SHUTDOWN_MS)).await;
        }

        tracing::info!(restart = i, txs_sent, "Triggering shutdown under load....");
        tokio::time::timeout(ROLLUP_SHUTDOWN_TIMEOUT, test_rollup.shutdown())
            .await
            .with_context(|| format!("shutdown timed out: restart={i} seed={seed}"))??;
        tracing::info!(restart = i, txs_sent, "Shutdown complete under load");
    }

    assert_only_known_logs_since(collector, log_start_idx);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_under_load_zk_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_under_load(
            OperatingMode::Zk,
            0,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_under_load_zk_non_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_under_load(
            OperatingMode::Zk,
            3,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_under_load_optimistic_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_under_load(
            OperatingMode::Optimistic,
            0,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_start_stop_under_load_optimistic_non_instant_finality() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false);
    for seed in [42, 1337] {
        start_stop_under_load(
            OperatingMode::Optimistic,
            3,
            RollupProverConfig::Skip,
            seed,
            &collector,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_start_prover_manual() -> anyhow::Result<()> {
    let collector = LogCollector::new(Level::WARN);
    initialize_logging_for_restart(collector.clone(), false); // Enable stdout logging. Set to false to disable.

    let rollup_storage_dir = Arc::new(tempfile::tempdir()?);
    let finalization_blocks = 0;

    let first_chunk = 6;
    let second_chunk = 4;
    let jump_size = first_chunk + second_chunk;

    let mock_da_dir = tempfile::tempdir()?;

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
        c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
        c.rollup_prover_config = Some(RollupProverConfig::Skip);
        // Since we have the prover enabled, we need to disable state root consistency checks.
        if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
            sequencer_conf.disable_state_root_consistency_checks = true;
            sequencer_conf.ideal_lag_behind_finalized_slot = 3;
        }
        c.aggregated_proof_block_jump = jump_size;
    })
    .set_da_config(|da_config| {
        da_config.connection_string = MockDaConfig::sqlite_in_dir(mock_da_dir.path()).unwrap();
    });

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
        (
            Level::WARN,
            "Metrics have been initialized outside of the rollup blueprint, some measurements can be lost on shutdown".to_string(),
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
        c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
        c.rollup_prover_config = Some(RollupProverConfig::Skip);
        c.aggregated_proof_block_jump = aggregated_proof_jump;
        c.max_channel_size = max_channel_size;
        c.max_infos_in_db = max_infos_in_db;
        if let sov_sequencer::SequencerKindConfig::Preferred(ref mut seq) = c.sequencer_config {
            seq.ideal_lag_behind_finalized_slot = 3;
        }
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
