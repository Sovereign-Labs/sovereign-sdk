#![allow(dead_code)]
use std::path::PathBuf;

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::U256;
use sov_demo_rollup::MockNomtDemoRollup;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SequencerKindConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{RollupBuilder, StoragePath, TestRollup};
use sov_evm::execution_config::EvmExecutionConfigContents;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_test_utils::SimpleStorage;
use tempfile::TempDir;

use crate::evm::evm_test_helper::alloy_client_with_signer;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
use crate::test_helpers::test_genesis_source;


/// Starts test rollup node.  
pub(crate) async fn start_node_with_ram_pinning(
    _rollup_prover_config: RollupProverConfig<Risc0>,
    location: TempDir,
    exec_config_path: PathBuf,
) -> TestRollup<MockNomtDemoRollup<Native>> {
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
    ).with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 32;
        c.rollup_prover_config = None; // reenable once sov-ethereum is compatible with proof blobs
        c.aggregated_proof_block_jump = 5;
        c.max_infos_in_db = 30;
        c.max_channel_size = 20;
        c.extension = Some(EVM_EXTENSION);
        if let SequencerKindConfig::Preferred( config) = &mut c.sequencer_config {
            config.num_cache_warmup_workers = 0;
        };
    })
    .start()
    .await
    .unwrap()
}


#[tokio::test(flavor = "multi_thread")]
async fn test_ram_pinning_config_updates() -> anyhow::Result<()> {
    sov_test_utils::initialize_logging();
    let temp_dir = tempfile::tempdir().unwrap();
    let exec_config_path = temp_dir.path().join("evm_execution_config.json");
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse().unwrap();
    let exec_config_contents = EvmExecutionConfigContents {
        default_bucket_size_limit: 100 * 1024 * 1024, // 100MB
        // privileged_deployer_addresses: vec![Address::from_str("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap()],
        privileged_deployer_addresses: vec![signer.address()],
        known_contracts_and_limits: Default::default(),
    };
    std::fs::write(&exec_config_path, serde_json::to_string_pretty(&exec_config_contents).unwrap()).unwrap();
    let test_rollup = start_node_with_ram_pinning(RollupProverConfig::Skip, temp_dir, exec_config_path.clone()).await;
    test_rollup.wait_for_next_blocks(3).await;
    let client = alloy_client_with_signer(test_rollup.http_addr, SENDER_PRIV_KEY);

    tracing::info!("Deploying contract");
    let contract = SimpleStorage::deploy(client).await?;
    let contract_address = contract.address();
    tracing::info!("Setting value");

    let tx = contract.set(U256::from(1)).send().await?;
    let _receipt = tx.get_receipt().await?;
    test_rollup.force_close_batch().await?;
    test_rollup.wait_for_next_blocks(1).await;

    tracing::info!("Getting value");
    let tx = contract.get().send().await?;
    let _receipt = tx.get_receipt().await?;


    let exec_config: EvmExecutionConfigContents = serde_json::from_str(&std::fs::read_to_string(&exec_config_path).unwrap()).unwrap();
    assert_eq!(exec_config.privileged_deployer_addresses, vec![signer.address()]);
    assert!(exec_config.known_contracts_and_limits.contains_key(contract_address), "Contract address should be in known contracts and limits");

    Ok(())
}
