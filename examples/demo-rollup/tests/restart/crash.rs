use crate::test_helpers::build_transfer_token_tx;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_api_spec::types;
use sov_bank::config_gas_token_id;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_cli::NodeClient;
use sov_db::test_utils::CrashLocation;
use sov_db::test_utils::CRASH_ENV_NAME;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockNomtDemoRollup;
use sov_demo_rollup::MockNomtRollupSpec;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::CryptoSpec;
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey;
use sov_modules_api::Spec;
use sov_sequencer::SeqConfigExtension;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::{RollupBuilder, StoragePath, TestRollup};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::Duration;

use crate::test_helpers::test_genesis_source;

// Starts test rollup node.
async fn start_node(location: Arc<TempDir>) -> TestRollup<MockNomtDemoRollup<Native>> {
    RollupBuilder::new(
        test_genesis_source(sov_modules_api::OperatingMode::Zk),
        BlockProducingConfig::Periodic {
            block_time_ms: 1000,
        },
        0,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.storage = StoragePath::Tmp(location);
        c.max_concurrent_blobs = 65536;
        c.rollup_prover_config = None;
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
        c.extension = Some(SeqConfigExtension {
            max_log_limit: 20000,
            response_size_limit: (1024 * 1024),
        });
    })
    .start()
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_saving_kernel_nomt() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeSavingKernelNomt),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_commiting_kernel_nomt() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeCommittingKernelNomt),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_saving_user_nomt() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeSavingUserlNomt),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_commiting_user_nomt() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeCommittingUserNomt),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_saving_ledger() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeSavingLedger),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_commiting_ledger() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeCommittingLedger),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_saving_archival() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeSavingArchival),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_comitting_archival() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeCommittingArchival),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_saving_live() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeSavingLive),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_crash_before_comitting_live() -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(120),
        test_start_stop_with_crash(CrashLocation::BeforeCommittingLive),
    )
    .await
    .unwrap()
}

// This test checks whether rollp can recover from different kinds of crashes, see `CrashLocation` enum.
async fn test_start_stop_with_crash(crash_moment: CrashLocation) -> anyhow::Result<()> {
    let temp_dir = Arc::new(tempfile::tempdir()?);
    let key_and_address =
        read_private_key::<MockNomtRollupSpec<Native>>("tx_signer_private_key.json");
    let receiver_addr = random_address::<MockNomtRollupSpec<Native>>();

    // Start the rollup for the first time, and after some transactions are received, crash it.
    {
        let test_rollup = start_node(temp_dir.clone()).await;
        test_rollup.wait_for_sequencer_ready().await.unwrap();

        let client = test_rollup.client.clone();

        let mut event_subscription = subscribe_to_bank_events(&test_rollup).await;
        let max_nb_of_txs = 500;

        println!("START1 =================");

        send_txs_in_background(
            0,
            receiver_addr,
            key_and_address.clone(),
            max_nb_of_txs,
            client,
        )
        .await;

        let mut nb_of_events = 0;
        loop {
            // Crash the node after 100 txs.
            if nb_of_events == 100 {
                crash_moment.set_crash_env();
            }

            // The subscription is closed once the node crashes.
            let next = event_subscription.next().await;
            if next.is_none() {
                assert!(!test_rollup.is_sequencer_ready().await);
                break;
            }

            nb_of_events += 1;
            assert!(
                nb_of_events < max_nb_of_txs,
                "The node didn't crash, but it was expected to."
            );
        }
    }

    // Give the OS time to clean up file handles after the crash.
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    std::env::remove_var(CRASH_ENV_NAME);
    unlock_dbs(&temp_dir);

    // Start the rollup with the existing DBs and check whether it is able to receive transactions.
    {
        let test_rollup = start_node(temp_dir).await;
        test_rollup.wait_for_sequencer_ready().await.unwrap();
        test_rollup.wait_for_next_blocks(10).await;

        println!("START2 =================");

        let client = test_rollup.client.clone();
        let mut event_subscription = subscribe_to_bank_events(&test_rollup).await;

        let max_nb_of_txs = 500;
        let start_generation = 1000;

        // Keep sending txs in the bacground.
        send_txs_in_background(
            start_generation,
            receiver_addr,
            key_and_address.clone(),
            max_nb_of_txs,
            client,
        )
        .await;

        // Check if transactions are coming through.
        for _ in 0..40 {
            let _ = event_subscription.next().await.unwrap().unwrap();
        }
        test_rollup.shutdown().await.unwrap();
    }

    Ok(())
}

fn unlock_dbs(temp_dir: &TempDir) {
    let lock_files = ["LOCK", "LOG", "LOG.old"];
    let dbs = ["state-db", "archival-state-db", "accessory", "blob_sender"];
    for lock_file in &lock_files {
        for db in &dbs {
            let _ = std::fs::remove_file(temp_dir.path().join(db).join(lock_file));
        }
    }
}

fn random_address<S: Spec>() -> <S as Spec>::Address {
    let pk = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    pk.pub_key().credential_id().into()
}

async fn send_txs_in_background(
    start_generation: u64,
    receiver: <MockNomtRollupSpec<Native> as Spec>::Address,
    key_and_address: PrivateKeyAndAddress<MockNomtRollupSpec<Native>>,
    max_nb_of_txs: u64,
    client: NodeClient,
) {
    tokio::spawn(async move {
        send_txs(
            start_generation,
            receiver,
            key_and_address,
            max_nb_of_txs,
            client,
        )
        .await;
    });
}

async fn send_txs(
    start_generation: u64,
    receiver: <MockNomtRollupSpec<Native> as Spec>::Address,
    key_and_address: PrivateKeyAndAddress<MockNomtRollupSpec<Native>>,
    max_nb_of_txs: u64,
    client: NodeClient,
) {
    let api_client = client.client.clone();
    let mut nb_of_txs = 0;
    loop {
        let tx = build_transfer_token_tx::<MockNomtRollupSpec<Native>>(
            &key_and_address.private_key,
            config_gas_token_id(),
            receiver,
            100,
            start_generation + nb_of_txs,
        );

        // It's fine not to check the result here — it will be verified later via subscription.
        let res = api_client.send_tx_to_sequencer(&tx).await;

        if res.is_err() {
            if std::env::var(CRASH_ENV_NAME).is_ok() {
                return;
            }
        }

        // Send transactions continuously every 100ms to maintain steady TX traffic during the test.
        tokio::time::sleep(Duration::from_millis(50)).await;
        nb_of_txs += 1;

        assert!(nb_of_txs < max_nb_of_txs);
    }
}

async fn subscribe_to_bank_events(
    test_rollup: &TestRollup<MockNomtDemoRollup<Native>>,
) -> BoxStream<'static, anyhow::Result<types::LedgerEvent>> {
    test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap()
}
