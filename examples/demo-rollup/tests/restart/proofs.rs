use std::net::SocketAddr;
use std::time::Duration;

use futures::StreamExt;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_full_node_configs::sequencer::SequencerKindConfig;
use sov_mock_da::storable::rpc::{start_server, MockDaClientConfig};
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaConfig};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::OperatingMode;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use sov_sequencer::preferred::RecoveryStrategy;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::test_rollup::{RollupBuilder, TestRollup};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
use tokio::sync::watch;

use crate::test_helpers::test_genesis_source;

const TEST_SEQ_DA_ADDRESS: MockAddress = MockAddress::new([0; 32]);
const AGGREGATED_PROOF_BLOCK_JUMP: usize = 3;
const PROOF_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const OFFLINE_DA_BLOCKS: u32 = 3;

async fn create_da_service_periodic() -> (StorableMockDaService, watch::Sender<()>, SocketAddr) {
    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(());
    let mut da_config = MockDaConfig::instant_with_sender(TEST_SEQ_DA_ADDRESS);
    da_config.block_producing = BlockProducingConfig::Periodic {
        block_time_ms: TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS * 2,
    };

    let da_service = StorableMockDaService::from_config(da_config, shutdown_receiver).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    (da_service, shutdown_sender, addr)
}

async fn build_rollup_with_prover(
    addr: SocketAddr,
) -> RollupBuilder<ExternalMockDemoRollup<Native>> {
    let genesis = test_genesis_source(OperatingMode::Zk);
    let da_config = MockDaClientConfig {
        url: format!("http://{addr}"),
    };
    RollupBuilder::new_with_external_da(genesis, da_config, None)
        .await
        .enable_prover()
        .set_config(|c| {
            c.blob_processing_timeout_secs = 300;
            c.aggregated_proof_block_jump = AGGREGATED_PROOF_BLOCK_JUMP;
            c.rollup_prover_config = RollupProverConfig::Prove;
            if let SequencerKindConfig::Preferred(p) = &mut c.sequencer_config {
                p.disable_state_root_consistency_checks = true;
                p.num_cache_warmup_workers = 0;
                p.recovery_strategy = RecoveryStrategy::TryToSave;
                p.ideal_lag_behind_finalized_slot = 3;
            }
        })
}

async fn wait_for_aggregated_proofs(
    test_rollup: &TestRollup<ExternalMockDemoRollup<Native>>,
    count: usize,
) {
    let api_client = test_rollup.api_client().clone();
    let mut proofs = api_client.subscribe_aggregated_proof().await.unwrap();
    for i in 0..count {
        let proof = tokio::time::timeout(PROOF_WAIT_TIMEOUT, proofs.next())
            .await
            .unwrap_or_else(|_| panic!("Timed out waiting for aggregated proof #{}", i + 1))
            .expect("Aggregated proof stream ended unexpectedly")
            .expect("Aggregated proof message was an error");
        tracing::info!(proof_index = i + 1, ?proof, "Received aggregated proof",);
    }
}

/// Integration test: external-DA rollup keeps processing aggregated proofs after a restart.
///
/// Scenario:
///   1. Start the rollup against an external mock DA, wait for 3 aggregated proofs.
///   2. Shut the rollup down for at least 3 DA blocks (the DA layer keeps producing while the rollup is offline).
///   3. Restart the rollup on the same storage and verify aggregated proofs are still being produced.
#[tokio::test(flavor = "multi_thread")]
async fn test_aggregated_proofs_after_restart_external_da() {
    //sov_test_utils::logging::initialize_or_change_logging_with_filter("info,tower=off");

    let (da_service, da_shutdown, addr) = create_da_service_periodic().await;
    // Give the DA layer some headroom so the rollup doesn't immediately starve on startup.
    da_service.wait_for_height(10).await.unwrap();

    // Phase 1: start the rollup and wait for 3 aggregated proofs.
    let test_rollup = build_rollup_with_prover(addr)
        .await
        .start_test_rollup()
        .await
        .unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    wait_for_aggregated_proofs(&test_rollup, 3).await;

    // Phase 2: shut the rollup down and wait for the DA to advance by at least 3 blocks.
    let height_at_shutdown = da_service.get_head_block_header().await.unwrap().height() as u32;

    println!("RESTART");

    let builder = test_rollup.shutdown().await.unwrap();

    da_service
        .wait_for_height(height_at_shutdown + OFFLINE_DA_BLOCKS)
        .await
        .unwrap();

    // Phase 3: restart on the same storage and confirm aggregated proofs keep flowing.
    let test_rollup = builder.start_test_rollup().await.unwrap();
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    wait_for_aggregated_proofs(&test_rollup, 9).await;

    let _ = test_rollup.shutdown().await;
    let _ = da_shutdown.send(());
}
