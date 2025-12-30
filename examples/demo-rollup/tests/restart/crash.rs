use crate::test_helpers::build_transfer_token_tx;
use futures::stream::BoxStream;
use futures::StreamExt;
use sov_api_spec::types;
use sov_bank::config_gas_token_id;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_cli::NodeClient;
use sov_db::test_utils::CrashMoment;
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
            block_time_ms: 1_000,
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

/// This test intentionally crashes the rollup during a commit to ensure that the correct state is computed afterward.
#[tokio::test(flavor = "multi_thread")]
async fn test_kernel_commit_crash() -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(30), test_start_stop_with_crash())
        .await
        .unwrap()
}

async fn test_start_stop_with_crash() -> anyhow::Result<()> {
    let temp_dir = Arc::new(tempfile::tempdir()?);
    let key_and_address =
        read_private_key::<MockNomtRollupSpec<Native>>("tx_signer_private_key.json");

    let receiver_addr = random_address::<MockNomtRollupSpec<Native>>();
    {
        let test_rollup = start_node(temp_dir.clone()).await;
        test_rollup.wait_for_sequencer_ready().await.unwrap();

        let client = test_rollup.client.clone();

        let mut event_subscription = subscribe_to_bank_events(&test_rollup).await;
        let max_nb_of_txs = 1000;

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
            // "Crash the node once the transactions are being processed."
            if nb_of_events == 5 {
                CrashMoment::BeforeCommittingUserNomt.set_crash_env();
            }

            // The subscription is closed once the node crashes.
            let next = event_subscription.next().await;
            if next.is_none() {
                assert!(!test_rollup.is_sequencer_ready().await);
                break;
            }

            nb_of_events += 1;
            assert!(nb_of_events < max_nb_of_txs);
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    std::env::remove_var(CRASH_ENV_NAME);
    unlock_dbs(&temp_dir);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    {
        let test_rollup = start_node(temp_dir).await;
        test_rollup.wait_for_sequencer_ready().await.unwrap();
        test_rollup.wait_for_next_blocks(10).await;

        let client = test_rollup.client.clone();
        let mut event_subscription = subscribe_to_bank_events(&test_rollup).await;

        let max_nb_of_txs = 100;
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

        // "Check if some transactions came through.
        for _ in 0..10 {
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

        let res = api_client.send_tx_to_sequencer(&tx).await;

        if res.is_err() {
            println!("res {:?}", res);
            // If the transaction fails, it should be due to the rollup crash.
            let x = std::env::var(CRASH_ENV_NAME);
            println!("=== XXXXX {x:?}");
            assert!(std::env::var(CRASH_ENV_NAME).is_ok());
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
        nb_of_txs += 1;

        assert!(nb_of_txs < max_nb_of_txs)
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
