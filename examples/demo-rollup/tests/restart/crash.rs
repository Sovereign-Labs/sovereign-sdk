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
use tempfile::TempDir;
use tokio::time::Duration;

use crate::test_helpers::test_genesis_source;

/// Starts test rollup node.  
pub(crate) async fn start_node_with_execution_config(
    location: TempDir,
) -> TestRollup<MockNomtDemoRollup<Native>> {
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

fn random_address<S: Spec>() -> <S as Spec>::Address {
    let pk = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    pk.pub_key().credential_id().into()
}

async fn send_txs_in_bg(
    start_nonce: u64,
    receiver: <MockNomtRollupSpec<Native> as Spec>::Address,
    client: NodeClient,
) {
    let key_and_address =
        read_private_key::<MockNomtRollupSpec<Native>>("tx_signer_private_key.json");

    tokio::spawn(async move {
        let mut n = 0;
        loop {
            let tx = build_transfer_token_tx::<MockNomtRollupSpec<Native>>(
                &key_and_address.private_key,
                config_gas_token_id(),
                receiver,
                100,
                start_nonce + n,
            );

            n += 1;

            client.client.send_tx_to_sequencer(&tx).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;

            if n == 20 {
                break;
            }
        }
    });
}

/// This test intentionally crashes the rollup during a commit to ensure that the correct state is computed afterward.
#[tokio::test(flavor = "multi_thread")]
async fn test_start_stop_with_crash() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let test_rollup = start_node_with_execution_config(temp_dir).await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let receiver_addr = random_address::<MockNomtRollupSpec<Native>>();

    let client = test_rollup.client.clone();

    let mut event_subscription = test_rollup
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    send_txs_in_bg(0, receiver_addr, client).await;

    for i in 0..100 {
        println!("X {}", i);
        tokio::time::timeout(Duration::from_millis(100), event_subscription.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    Ok(())
}
