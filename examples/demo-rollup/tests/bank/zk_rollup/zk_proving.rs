use std::time::Duration;

use anyhow::Context;
use demo_stf::genesis_config::create_genesis_config;
use futures::StreamExt;
use sov_modules_api::OperatingMode;
use sov_modules_stf_blueprint::GenesisParams;
use sov_test_utils::test_rollup::GenesisSource;

use crate::bank::helpers::*;
use crate::test_helpers::{test_genesis_paths, DemoRollupSpec};

const AGGREGATION_GATE_ENV: &str = "SOV_MOCK_AGGREGATION_GATE";

fn stop_proving() {
    std::env::set_var(AGGREGATION_GATE_ENV, "STOP_PROVING");
}

fn resume_proving() {
    std::env::remove_var(AGGREGATION_GATE_ENV);
}

#[tokio::test(flavor = "multi_thread")]
async fn max_concurrent_proof_with_nonzero_genesis_da_height() -> anyhow::Result<()> {
    let test_case: TestCase = TestCase {
        wait_for_aggregated_proof: true,
        finalization_blocks: 0,
        aggregated_proof_block_jump: 2,
        max_concurrent_proof_blobs: 2,
    };

    let operating_mode = OperatingMode::Zk;
    let mut runtime_config =
        create_genesis_config::<DemoRollupSpec>(&test_genesis_paths(operating_mode))?;
    runtime_config.chain_state.genesis_da_height = 5;
    let genesis = GenesisSource::CustomParams(GenesisParams {
        runtime: runtime_config,
    });

    let test_rollup = start_test_rollup_with_genesis(&test_case, operating_mode, genesis).await?;

    let mut aggregated_proof_subscription = test_rollup
        .client
        .client
        .subscribe_aggregated_proof()
        .await
        .context("Failed to subscribe to aggregated proof")?;

    let _ = aggregated_proof_subscription.next().await.unwrap().unwrap();

    stop_proving();
    let mut finalized_slots = test_rollup
        .client
        .client
        .subscribe_finalized_slots()
        .await
        .context("Failed to subscribe to finalized slots")?;

    for _ in 0..20 {
        let _ = finalized_slots.next().await.unwrap()?;
    }
    resume_proving();
    test_rollup
        .wait_for_rollup_to_shutdown(Duration::from_secs(10))
        .await;

    Ok(())
}
