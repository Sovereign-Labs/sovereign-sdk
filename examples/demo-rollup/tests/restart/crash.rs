use std::path::PathBuf;

use alloy::signers::local::PrivateKeySigner;
use alloy_provider::Provider;
use sov_demo_rollup::mock_da_risc0_host_args;
use sov_demo_rollup::MockNomtDemoRollup;
use sov_evm::execution_config::EvmExecutionConfigContents;
use sov_evm_test_utils::SimpleStorage;
use sov_evm_test_utils::Submit;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::execution_mode::Native;
use sov_risc0_adapter::Risc0;
use sov_sequencer::SeqConfigExtension;
use sov_sequencer::SequencerKindConfig;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{RollupBuilder, StoragePath, TestRollup};
use tempfile::TempDir;

//use crate::evm::evm_test_helper::alloy_client_with_signer;
//use crate::evm::evm_test_helper::EVM_EXTENSION;
//use crate::evm::evm_test_helper::SENDER_PRIV_KEY;
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

/// This test intentionally crashes the rollup during a commit to ensure that the correct state is computed afterward.
#[tokio::test(flavor = "multi_thread")]
async fn test_start_stop_with_crash() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;

    let test_rollup = start_node_with_execution_config(temp_dir).await;
    test_rollup.wait_for_next_blocks(1).await;

    Ok(())
}
