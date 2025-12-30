use std::path::PathBuf;

use crate::test_helpers::build_transfer_token_tx;
use alloy::signers::local::PrivateKeySigner;
use alloy_provider::Provider;
use futures::StreamExt;
use sov_bank::config_gas_token_id;
use sov_cli::wallet_state::PrivateKeyAndAddress;
use sov_cli::NodeClient;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockNomtDemoRollup;
use sov_demo_rollup::MockNomtRollupSpec;
use sov_evm::execution_config::EvmExecutionConfigContents;
use sov_evm_test_utils::SimpleStorage;
use sov_evm_test_utils::Submit;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::CryptoSpec;
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey;
use sov_modules_api::Spec;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SeqConfigExtension;
use sov_sequencer::SequencerKindConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::{RollupBuilder, StoragePath, TestRollup};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::Duration;

use crate::test_helpers::test_genesis_source;

/// Starts test rollup node.  
pub(crate) async fn start_node(location: Arc<TempDir>) -> TestRollup<MockNomtDemoRollup<Native>> {
    // Don't provide a prover since the EVM is not currently provable
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
async fn test_start_stop_with_crash() -> anyhow::Result<()> {
    let temp_dir = Arc::new(tempfile::tempdir()?);
    let key_and_address =
        read_private_key::<MockNomtRollupSpec<Native>>("tx_signer_private_key.json");

    let pub_key = key_and_address.private_key.pub_key();
    let receiver_addr = random_address::<MockNomtRollupSpec<Native>>();

    {
        let test_rollup = start_node(temp_dir.clone()).await;
        test_rollup.wait_for_sequencer_ready().await.unwrap();

        let client = test_rollup.client.clone();

        let mut event_subscription = test_rollup
            .api_client()
            .subscribe_to_events_with_filter("Bank/*")
            .await
            .unwrap();

        send_txs_in_bg(0, receiver_addr, key_and_address.clone(), None, client).await;

        for i in 0.. {
            if i == 5 {
                std::env::set_var("SOV_CRASH_ON_COMMIT", "1");
            }

            let next = tokio::time::timeout(Duration::from_millis(200), event_subscription.next())
                .await
                .unwrap();

            if next.is_none() {
                break;
            }
        }
    }

    std::env::remove_var("SOV_CRASH_ON_COMMIT");
    unlock_dbs(&temp_dir);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    {
        let test_rollup = start_node(temp_dir).await;
        test_rollup.wait_for_sequencer_ready().await.unwrap();
        test_rollup.wait_for_next_blocks(10).await;

        let client = test_rollup.client.clone();

        let mut event_subscription = test_rollup
            .api_client()
            .subscribe_to_events_with_filter("Bank/*")
            .await
            .unwrap();

        let start_nonce = 100;

        let max_nb_of_txs = 10;

        send_txs_in_bg(
            start_nonce + 1,
            receiver_addr,
            key_and_address.clone(),
            Some(max_nb_of_txs),
            client,
        )
        .await;

        for i in 0..max_nb_of_txs {
            let res = tokio::time::timeout(Duration::from_millis(200), event_subscription.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
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

async fn send_txs_in_bg(
    start_nonce: u64,
    receiver: <MockNomtRollupSpec<Native> as Spec>::Address,
    key_and_address: PrivateKeyAndAddress<MockNomtRollupSpec<Native>>,
    max_nb_of_txs: Option<u64>,
    client: NodeClient,
) {
    tokio::spawn(async move {
        send_txs(
            start_nonce,
            receiver,
            key_and_address,
            max_nb_of_txs,
            client,
        )
        .await;
    });
}

async fn send_txs(
    start_nonce: u64,
    receiver: <MockNomtRollupSpec<Native> as Spec>::Address,
    key_and_address: PrivateKeyAndAddress<MockNomtRollupSpec<Native>>,
    max_nb_of_txs: Option<u64>,
    client: NodeClient,
) {
    let mut nb_of_txs = 0;
    loop {
        let tx = build_transfer_token_tx::<MockNomtRollupSpec<Native>>(
            &key_and_address.private_key,
            config_gas_token_id(),
            receiver,
            100,
            start_nonce + nb_of_txs,
        );

        let res = client.client.send_tx_to_sequencer(&tx).await;

        println!("XXX {max_nb_of_txs:?}");
        if max_nb_of_txs.is_some() {
            println!("XXX {res:?}");
        }

        if res.is_err() {
            assert!(std::env::var("SOV_CRASH_ON_COMMIT").is_ok());
        }

        if let Some(max_nb_of_txs) = max_nb_of_txs {
            if nb_of_txs >= max_nb_of_txs {
                println!();
                println!("OO {max_nb_of_txs:?}  {nb_of_txs:?}");

                println!("Nonce: {res:?}");
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
        nb_of_txs += 1;
    }
}
