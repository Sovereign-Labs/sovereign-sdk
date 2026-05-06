mod max_concurrent_proof_blobs;

use futures::StreamExt;
use sov_demo_rollup::MockDemoRollup;
use sov_full_node_configs::sequencer::{RecoveryStrategy, SequencerKindConfig};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::OperatingMode;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{RollupBuilder, TestRollup};
use sov_test_utils::{
    TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS, TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
};

use crate::test_helpers::test_genesis_source;

/// Single place for configuring test rollup.
/// Applies all necessary configuration changes to make it work with the tests.
/// Starts it and ensures it is ready to accept transactions.
pub async fn start_test_rollup() -> anyhow::Result<TestRollup<MockDemoRollup<Native>>> {
    let test_rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Zk),
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        0,
    )
    .enable_prover()
    .set_config(|c| {
        c.max_concurrent_batch_blobs = 16777216;
        c.rollup_prover_config = RollupProverConfig::Prove;
        c.blob_processing_timeout_secs = 180;
        c.aggregated_proof_block_jump = 2;
        c.max_concurrent_proof_blobs = 2;
        if let SequencerKindConfig::Preferred(sequencer_config) = &mut c.sequencer_config {
            sequencer_config.batch_execution_time_limit_millis = TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
            sequencer_config.recovery_strategy = RecoveryStrategy::TryToSave;
            sequencer_config.disable_state_root_consistency_checks = true;
            sequencer_config.num_cache_warmup_workers = 0;
            sequencer_config.ideal_lag_behind_finalized_slot = 3;
        }
    })
    .start()
    .await?;

    // We need a handful of blocks for the sequencer to be able to advance the
    // visible slot number.
    let warm_up_blocks = 5;

    let mut slots = test_rollup.client.client.subscribe_slots().await?;
    for _ in 0..warm_up_blocks {
        let _slot = slots.next().await;
    }

    test_rollup.wait_for_sequencer_ready().await?;

    Ok(test_rollup)
}
