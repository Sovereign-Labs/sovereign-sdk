//! Tests for shutdown/restart cases.
use std::collections::HashSet;
use std::env;
use std::str::FromStr;
use std::sync::Arc;

use crate::test_helpers::test_genesis_source;
use anyhow::Context;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use futures::StreamExt;
use rand::Rng;
use sov_api_spec::types as api_types;
use sov_bank::{config_gas_token_id, Coins};
use sov_db::storage_manager::NomtStorageManager;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::BlockProducingConfig;
use sov_mock_da::MockHash;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Amount;
use sov_modules_api::CryptoSpec;
use sov_modules_api::DispatchCall;
use sov_modules_api::OperatingMode;
use sov_modules_api::PrivateKey;
use sov_modules_api::RawTx;
use sov_modules_api::Runtime;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::logging::default_rust_log_value;
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::ledger_api::IncludeChildren;
use sov_sequencer::SequencerKindConfig;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::DefaultStorageSpec;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::default_test_signed_transaction;
use sov_test_utils::generate_operator_runtime_with_kernel;
use sov_test_utils::logging::LogCollector;
use sov_test_utils::runtime::genesis::operator::HighLevelOperatorGenesisConfig;
use sov_test_utils::runtime::GenesisParams;
use sov_test_utils::test_rollup::GenesisSource;
use sov_test_utils::test_rollup::{RollupBuilder, StoragePath, TestRollup};
use sov_test_utils::MockDaSpec;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::TestNomtSpec as TestSpec;
use sov_test_utils::TestPrivateKey;
use sov_test_utils::TestUser;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;
use tracing::Level;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, registry, EnvFilter, Layer};

generate_operator_runtime_with_kernel!(
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    TestRuntime <=
);

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
    let new_env_filter = EnvFilter::from_str("debug,jmt=warn")?;
    let fmt_layer = fmt::layer().with_filter(new_env_filter);
    let subscriber = registry().with(fmt_layer).with(collector.clone());
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
                c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
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

        tracing::info!("Triggering shutdown....");
        tokio::time::timeout(ROLLUP_SHUTDOWN_TIMEOUT, test_rollup.shutdown()).await??;
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
        (Level::WARN, "The node is unsynced and doesn't know it. This probably means that you wiped the node DB and are resyncing.".to_string()),
        (Level::WARN, "Metics have been initialized outside of the rollup blueprint, some measurements can be lost on shutdown".to_string()),
    ];

    let mut recorded_errors_warnings =
        HashSet::<(Level, String)>::from_iter(collector.records().iter().cloned());
    recorded_errors_warnings.retain(|e| !known.contains(e));
    // We could've checked `.is_empty`, but in case of failure, we will see errors immediately.
    assert_eq!(HashSet::<(Level, String)>::new(), recorded_errors_warnings);
    Ok(())
}

type StorageManager = NomtStorageManager<
    MockDaSpec,
    <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher,
    NomtProverStorage<
        DefaultStorageSpec<<<TestSpec as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
        MockHash,
    >,
>;

/// This test intentionally crashes the rollup during a commit to ensure that the correct state is computed afterward.
#[tokio::test(flavor = "multi_thread")]
async fn test_start_stop_with_crash() -> anyhow::Result<()> {
    // Use a very large balance so that gas can be safely ignored.
    const INITIAL_BALANCE: u128 = 1000000000000000000;
    let reward_user =
        TestUser::<TestSpec>::new(TestPrivateKey::generate(), Amount::new(INITIAL_BALANCE));
    let genesis_config = HighLevelOperatorGenesisConfig::generate(reward_user)
        .add_additional_accounts(1, Amount::new(INITIAL_BALANCE));
    let admin = genesis_config.additional_accounts()[1].clone();

    let rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
        );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };
    let sequencer_address = genesis_params
        .runtime
        .sequencer_registry
        .sequencer_config
        .seq_da_address;
    let rollup_storage_dir = Arc::new(tempfile::tempdir()?);

    let test_rollup = tokio::time::timeout(
        ROLLUP_START_TIMEOUT,
        RollupBuilder::<RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>, StorageManager>>::new(
            GenesisSource::CustomParams(genesis_params.clone()),
            BlockProducingConfig::Manual,
            0,
        )
        .set_config(|c| {
            c.max_concurrent_blobs = 65536;
            c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
            if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
                sequencer_conf.disable_state_root_consistency_checks = true;
            }
            c.aggregated_proof_block_jump = 40;
            c.separate_archival_db = true;
        })
        .set_da_config(|c| c.sender_address = sequencer_address)
        .set_persistent_da()
        .start(),
    )
    .await
    .context("Starting rollup failed")??;

    // Wait for rollup to start
    let mut slot_subscription = test_rollup
        .client
        .client
        .subscribe_slots_with_children(IncludeChildren::new(true))
        .await?;
    // TODO: Why 5 blocks specifically, and not 3 or 10?
    for _ in 0..5 {
        test_rollup.da_service.produce_block_now().await?;
        let _ = slot_subscription.next().await.unwrap().unwrap();
    }

    // Send enough that each tx will change the leading digit of our balance. Use a large buffer so that gas can be ignored.
    const AMOUNT_TO_SEND: u128 = (INITIAL_BALANCE / 100) * 9;

    // Send some transactions in a loop. This ensures that each slot has a batch, so there will be a non-empty db update when we eventually crash.
    for i in 6..=9 {
        let tx = tx_send_transfer(AMOUNT_TO_SEND, admin.private_key(), i);
        test_rollup
            .api_client()
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap();
        test_rollup.force_close_batch().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        test_rollup.da_service.produce_block_now().await.unwrap();
        let slot = slot_subscription.next().await.unwrap().unwrap();
        assert_eq!(slot.number, i);
        assert_eq!(slot.batches[0].tx_range.end, i - 5);

        let response = test_rollup
            .client
            .get_balance::<TestSpec>(&admin.address(), &config_gas_token_id(), None)
            .await?;

        assert!(
            response.to_string().starts_with(&(9 - (i - 6)).to_string()),
            "Balance after set-value: {response}",
        );
    }

    // Sleep to ensure that any pending commits have finished
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // TODO: wait for expected slot + wait for sequencer readiness

    // Send one more transaction which should crash on commit.
    {
        std::env::set_var("SOV_CRASH_ON_COMMIT", "1");
        let tx = tx_send_transfer(AMOUNT_TO_SEND, admin.private_key(), 10);
        test_rollup
            .api_client()
            .send_raw_tx_to_sequencer(&tx)
            .await?;
        test_rollup.force_close_batch().await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        // Produce 2 blocks, just to make sure that finalization is happening
        test_rollup.da_service.produce_n_blocks_now(2).await?;
        test_rollup
            .wait_for_rollup_to_crash(std::time::Duration::from_secs(10))
            .await?;
    }
    std::env::remove_var("SOV_CRASH_ON_COMMIT");

    // Give the OS time to clean up file handles after the crash
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Clean up RocksDB lock files that may remain after crashing
    let lock_files = ["LOCK", "LOG", "LOG.old"];
    let dbs = ["state-db", "archival-state-db", "accessory", "blob_sender"];
    for lock_file in &lock_files {
        for db in &dbs {
            // Ignore any errors
            let _ = std::fs::remove_file(rollup_storage_dir.path().join(db).join(lock_file));
        }
    }

    // Restart the rollup and check that no writes have been lost due to the crash on commit.
    let test_rollup = tokio::time::timeout(
        ROLLUP_START_TIMEOUT,
        RollupBuilder::<RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>, StorageManager>>::new(
            GenesisSource::CustomParams(genesis_params.clone()),
            BlockProducingConfig::Manual,
            0,
        )
        .set_config(|c| {
            c.max_concurrent_blobs = 65536;
            c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
            if let SequencerKindConfig::Preferred(sequencer_conf) = &mut c.sequencer_config {
                sequencer_conf.disable_state_root_consistency_checks = true;
            }
            c.aggregated_proof_block_jump = 40;
            c.separate_archival_db = true;
        })
        .set_da_config(|c| c.sender_address = sequencer_address)
        .set_persistent_da()
        .start(),
    )
    .await
    .context("Starting rollup failed")??;

    let mut slot_subscription = test_rollup
        .client
        .client
        .subscribe_slots_with_children(IncludeChildren::new(true))
        .await?;
    for _ in 0..2 {
        test_rollup.da_service.produce_block_now().await?;
        let _ = slot_subscription.next().await.unwrap().unwrap();
    }

    let response = test_rollup
        .client
        .get_balance::<TestSpec>(&admin.address(), &config_gas_token_id(), None)
        .await?;

    // Check that the balance is what we expect - i.e. no writes have been lost due to the crash on commit.
    assert!(
        response.to_string().starts_with("5"),
        "Balance after set-value: {response}",
    );
    Ok(())
}

fn tx_send_transfer(value_to_set: u128, key: &Ed25519PrivateKey, nonce: u64) -> RawTx {
    let msg =
        <TestRuntime<TestSpec> as DispatchCall>::Decodable::Bank(sov_bank::CallMessage::Transfer {
            to: "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
                .parse()
                .unwrap(),
            coins: Coins {
                amount: Amount::new(value_to_set),
                token_id: config_gas_token_id(),
            },
        });
    let tx = default_test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        &msg,
        nonce,
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
    );
    RawTx::new(borsh::to_vec(&tx).unwrap())
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
        c.storage = StoragePath::Tmp(rollup_storage_dir.clone());
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
        (
            Level::WARN,
            "Metics have been initialized outside of the rollup blueprint, some measurements can be lost on shutdown".to_string(),
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
