use std::path::PathBuf;

use alloy::signers::local::PrivateKeySigner;
use alloy_provider::Provider;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockDemoRollup;
use sov_evm::execution_config::EvmExecutionConfigContents;
use sov_evm_test_utils::SimpleStorage;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SequencerKindConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{RollupBuilder, StoragePath, TestRollup};
use tempfile::TempDir;

use crate::evm::evm_test_helper::alloy_client_with_signer;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use crate::test_helpers::test_genesis_source;

/// Starts test rollup node.  
pub(crate) async fn start_node_with_execution_config(
    _rollup_prover_config: RollupProverConfig<Risc0>,
    location: TempDir,
    exec_config_path: PathBuf,
) -> TestRollup<MockDemoRollup<Native>> {
    let storage_path = StoragePath::Tmp(std::sync::Arc::new(location));
    // Don't provide a prover since the EVM is not currently provable
    RollupBuilder::new_with_storage_path_and_exec_config(
        test_genesis_source(sov_modules_api::OperatingMode::Zk),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        0,
        storage_path,
        false,
        Some(exec_config_path),
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 32;
        c.rollup_prover_config = None; // reenable once sov-ethereum is compatible with proof blobs
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
        c.extension = Some(EVM_EXTENSION);
        if let SequencerKindConfig::Preferred(config) = &mut c.sequencer_config {
            config.num_cache_warmup_workers = 0;
            config.ideal_lag_behind_finalized_slot = 3;
        };
    })
    .start()
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_allow_publishing_reverted_txs() -> anyhow::Result<()> {
    do_revert_tx_test(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_disable_publishing_reverted_txs() -> anyhow::Result<()> {
    do_revert_tx_test(false).await
}

async fn do_revert_tx_test(preferred_sequencer_publish_reverted_txs: bool) -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let exec_config_path = temp_dir.path().join("evm_execution_config.json");
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;
    let exec_config_contents = EvmExecutionConfigContents {
        preferred_sequencer_publish_reverted_txs,
        ..Default::default()
    };
    std::fs::write(
        &exec_config_path,
        serde_json::to_string_pretty(&exec_config_contents)?,
    )?;
    let test_rollup = start_node_with_execution_config(
        RollupProverConfig::Skip,
        temp_dir,
        exec_config_path.clone(),
    )
    .await;
    test_rollup.wait_for_rollup_height_advance_by(1).await;
    let client = alloy_client_with_signer(test_rollup.http_addr, SENDER_PRIV_KEY);

    let rpc_nonce_before = client.get_transaction_count(signer.address()).await?;
    let contract = SimpleStorage::deploy(client.clone()).await?;
    let nonce = client.get_transaction_count(signer.address()).await?;
    let rpc_nonce_after_deploy = client.get_transaction_count(signer.address()).await?;
    assert_eq!(rpc_nonce_before, 0);
    assert_eq!(nonce, 1);
    assert_eq!(rpc_nonce_after_deploy, 1);
    let exec_config: EvmExecutionConfigContents =
        serde_json::from_str(&std::fs::read_to_string(&exec_config_path)?)?;
    assert_eq!(
        exec_config.preferred_sequencer_publish_reverted_txs,
        preferred_sequencer_publish_reverted_txs,
    );

    // Set explicit gas to avoid pre-submit `eth_estimateGas`, which now correctly
    // errors on reverting calls.
    let result = contract.alwaysRevert().gas(300_000).send().await;
    let nonce = client.get_transaction_count(signer.address()).await?;
    let rpc_nonce_after_revert = client.get_transaction_count(signer.address()).await?;
    if preferred_sequencer_publish_reverted_txs {
        let pending_tx = result?;
        let receipt = pending_tx.get_receipt().await?;
        // TC05: Reverted transaction receipt must have status=0x0
        assert!(
            !receipt.status(),
            "reverted transaction must have status=0x0"
        );
        assert_eq!(nonce, 2);
        assert_eq!(rpc_nonce_after_revert, 2);
    } else {
        assert!(result.is_err());
        assert_eq!(nonce, 1);
        assert_eq!(rpc_nonce_after_revert, 1);
    }
    Ok(())
}
