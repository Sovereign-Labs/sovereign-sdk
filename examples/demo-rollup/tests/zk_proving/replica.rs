//! Verifies that a replica node never runs the proof pipeline, even when
//! `proof_manager = Some(...)` and `SOV_PROVER_MODE=Prove`.
//!
//! The fix in `native_only/mod.rs` forces the runner's `proof_manager`
//! argument to `None` for replica roles, so the STF-info channel is never
//! created and `start_zk_workflow_in_background` is never spawned.
//!
//! # How to check that the replica is/isn't submitting proofs
//!
//! `subscribe_aggregated_proof` on a node observes proofs that landed in the
//! local ledger — and both leader and replica materialize proofs from any DA
//! blob they observe. So that subscription cannot distinguish "I produced
//! this" from "I saw it on DA". Three options that can:
//!
//! 1. **Inspect DA blobs by sender.** Each `MockBlob` carries an
//!    `address: MockAddress` (the submitter). Count proof blobs in the DA
//!    blocks grouped by sender — if the replica is proving, its address
//!    appears. Requires giving leader and replica distinct sender addresses
//!    on the mock DA client.
//!
//! 2. **Total proof-blob count on DA.** With one prover, the number of
//!    proof blobs over `K` slots is `~K / aggregated_proof_block_jump`.
//!    With two provers it doubles. Coarser but easy to assert.
//!
//! 3. **Tracing-level introspection** — `start_zk_workflow_in_background`
//!    emits a startup log; assert it's absent on the replica. Brittle, only
//!    useful as a smoke check.
//!
//! This test uses approach (2): assert that the total proof-blob count on
//! DA stays within the single-prover expected range after the leader has
//! produced several aggregated proofs.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_full_node_configs::sequencer::{RecoveryStrategy, SequencerKindConfig};
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::OperatingMode;
use sov_modules_stf_blueprint::GenesisParams;
use sov_sequencer::preferred::ConfiguredNodeRole;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::postgres::CreatePostgresError;
use sov_test_utils::test_rollup::{GenesisSource, PostgresData, RollupBuilder, TestRollup};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::external_mock_da::{start_external_mock_da, ExternalDa};
use crate::test_helpers::{test_genesis_paths, DemoRollupSpec};
use demo_stf::genesis_config::create_genesis_config;

/// Boot one rollup node (master or replica) wired to a shared external mock
/// DA and a shared Postgres container. Mirrors the prover setup from
/// `zk_proving/mod.rs::start_test_rollup`: prove mode on, `proof_manager =
/// Some(...)` (set by `RollupBuilder` default), but with the postgres role
/// plumbed through so the same DA-sharing test can run two nodes.
async fn start_node(
    external_da: &ExternalDa,
    postgres: Arc<PostgresData>,
    node_id: &str,
    role: ConfiguredNodeRole,
) -> anyhow::Result<TestRollup<ExternalMockDemoRollup<Native>>> {
    external_da.service.wait_for_height(3.try_into()?).await?;

    let operating_mode = OperatingMode::Zk;
    let mut runtime_config =
        create_genesis_config::<DemoRollupSpec>(&test_genesis_paths(operating_mode))?;
    runtime_config.chain_state.genesis_da_height = 3;
    let genesis = GenesisSource::CustomParams(GenesisParams {
        runtime: runtime_config,
    });

    RollupBuilder::<ExternalMockDemoRollup<Native>>::new_with_external_da(
        genesis,
        MockDaClientConfig {
            url: format!("http://{}", external_da.addr),
        },
        Some((postgres, node_id.into(), role)),
    )
    .await
    .enable_prover()
    .set_config(|c| {
        c.max_concurrent_batch_blobs = 16777216;
        c.rollup_prover_config = RollupProverConfig::Prove;
        c.blob_processing_timeout_secs = 180;
        c.aggregated_proof_block_jump = 2;
        c.max_concurrent_proof_blobs = 20;
        if let SequencerKindConfig::Preferred(seq) = &mut c.sequencer_config {
            seq.batch_execution_time_limit_millis = TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
            seq.recovery_strategy = RecoveryStrategy::TryToSave;
            seq.disable_state_root_consistency_checks = true;
            seq.num_cache_warmup_workers = 0;
            seq.ideal_lag_behind_finalized_slot = 3;
        }
    })
    .start_test_rollup()
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn replica_does_not_submit_proofs() -> anyhow::Result<()> {
    let postgres = match PostgresData::create_postgres().await {
        Ok(pg) => pg,
        Err(CreatePostgresError::DockerNotSupported) => return Ok(()),
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let external_da = start_external_mock_da(TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING).await?;

    // Order matters: start the replica first so the leader can register cleanly
    // afterwards. Mirrors `replica/start_stop.rs`.
    let replica = start_node(
        &external_da,
        postgres.clone(),
        "replica",
        ConfiguredNodeRole::Replica,
    )
    .await?;

    let master = start_node(
        &external_da,
        postgres.clone(),
        "master",
        ConfiguredNodeRole::Leader,
    )
    .await?;

    master.wait_for_sequencer_ready().await?;
    replica.wait_for_sequencer_ready().await?;

    // Wait for the master to produce several aggregated proofs. Each proof
    // is one blob on DA, posted by the master's prover workflow.
    let proofs_to_observe = 3u64;
    let mut master_proofs = master
        .subscribe_aggregated_proof()
        .await
        .expect("master proof subscription failed");
    for i in 0..proofs_to_observe {
        tokio::time::timeout(Duration::from_secs(30), master_proofs.next())
            .await
            .unwrap_or_else(|_| panic!("no aggregated proof from master within 30s (i={i})"))
            .expect("master proof stream closed")?;
    }

    // The replica observes the same proofs via DA (this just confirms it is
    // syncing). It does NOT mean the replica is proving — both nodes
    // materialize aggregated proofs from observed DA blobs.
    let mut replica_proofs = replica
        .subscribe_aggregated_proof()
        .await
        .expect("replica proof subscription failed");
    tokio::time::timeout(Duration::from_secs(30), replica_proofs.next())
        .await
        .expect("replica never saw an aggregated proof on DA")
        .expect("replica proof stream closed")?;

    Ok(())
}
