mod max_concurrent_proof_blobs;
mod skip_proving_on_resync;

use demo_stf::genesis_config::create_genesis_config;
use futures::StreamExt;
use serde::Deserialize;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_full_node_configs::sequencer::{RecoveryStrategy, SequencerKindConfig};
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::AggregatedProofPublicData;
use sov_modules_api::OperatingMode;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_modules_stf_blueprint::GenesisParams;
use sov_state::Storage;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;

use crate::external_mock_da::ExternalDa;
use crate::test_helpers::{test_genesis_paths, DemoRollupSpec};

type RollupSpec = <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
type ProofPublicData = AggregatedProofPublicData<
    <RollupSpec as Spec>::Address,
    <RollupSpec as Spec>::Da,
    <<RollupSpec as Spec>::Storage as Storage>::Root,
>;

/// Single place for configuring test rollup.
/// Applies all necessary configuration changes to make it work with the tests.
/// Starts it and ensures it is ready to accept transactions.
///
/// Connects the rollup to the provided external mock DA service.
/// `genesis_da_height` overrides the value found in `chain_state.json`.
pub async fn start_test_rollup(
    genesis_da_height: u64,
    external_da: &ExternalDa,
    max_concurrent_proof_blobs: usize,
    start_fresh_outer_proof_on_resync: bool,
) -> anyhow::Result<TestRollup<ExternalMockDemoRollup<Native>>> {
    // Make sure the DA has produced the genesis block before the rollup tries
    // to read from it.
    external_da
        .service
        .wait_for_height(genesis_da_height.try_into()?)
        .await?;

    let operating_mode = OperatingMode::Zk;
    let mut runtime_config =
        create_genesis_config::<DemoRollupSpec>(&test_genesis_paths(operating_mode))?;
    runtime_config.chain_state.genesis_da_height = genesis_da_height;
    let genesis = GenesisSource::CustomParams(GenesisParams {
        runtime: runtime_config,
    });

    let test_rollup = RollupBuilder::<ExternalMockDemoRollup<Native>>::new_with_external_da(
        genesis,
        MockDaClientConfig {
            url: format!("http://{}", external_da.addr),
        },
        None,
    )
    .await
    .enable_prover()
    .set_config(|c| {
        c.max_concurrent_batch_blobs = 16777216;
        c.rollup_prover_config = RollupProverConfig::Prove;
        c.blob_processing_timeout_secs = 180;
        c.aggregated_proof_block_jump = 2;
        c.max_concurrent_proof_blobs = max_concurrent_proof_blobs;
        c.start_fresh_outer_proof_on_resync = start_fresh_outer_proof_on_resync;
        if let SequencerKindConfig::Preferred(sequencer_config) = &mut c.sequencer_config {
            sequencer_config.batch_execution_time_limit_millis = TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
            sequencer_config.recovery_strategy = RecoveryStrategy::TryToSave;
            sequencer_config.disable_state_root_consistency_checks = true;
            sequencer_config.num_cache_warmup_workers = 0;
            sequencer_config.ideal_lag_behind_finalized_slot = 3;
        }
    })
    .start_test_rollup()
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

#[derive(Debug, Deserialize)]
struct ValueResponse {
    value: ProofPublicData,
}

pub async fn query_verified_proofs(
    test_rollup: &TestRollup<ExternalMockDemoRollup<Native>>,
) -> anyhow::Result<ProofPublicData> {
    let value = test_rollup
        .client
        .query_rest_endpoint::<ValueResponse>(
            "/modules/prover-incentives/state/latest-proof-succesfully-verified",
        )
        .await?;

    Ok(value.value)
}
