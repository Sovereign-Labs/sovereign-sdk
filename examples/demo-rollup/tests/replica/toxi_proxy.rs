//! ToxiProxy-based network chaos tests for leader-replica failover scenarios.
//!
//! These tests use ToxiProxy to simulate network failures between the rollup nodes
//! and PostgreSQL, verifying that leader election and failover work correctly.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use sov_bank::config_gas_token_id;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_full_node_configs::sequencer::SequencerKindConfig;
use sov_mock_da::storable::rpc::{start_server, MockDaClientConfig};
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CryptoSpec, OperatingMode, PrivateKey, PublicKey, Spec};
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_proxy_utils::{ClusterInfo, NodeDiscovery};
use sov_sequencer::preferred::ConfiguredNodeRole;
use sov_sequencer::SequencerRole;
use sov_test_utils::postgres::CreatePostgresError;
use sov_test_utils::test_rollup::{read_private_key, RollupBuilder, TestRollup};
use sov_test_utils::toxiproxy::ToxiProxyPostgresData;
use sov_test_utils::TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::test_helpers::{build_transfer_token_tx, test_genesis_source};

type S = <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;
type Rollup = ExternalMockDemoRollup<Native>;

const TEST_SEQ_DA_ADDRESS: MockAddress = MockAddress::new([0; 32]);

fn random_address() -> <S as Spec>::Address {
    let pk = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    pk.pub_key().credential_id().into()
}

async fn create_da_service_periodic() -> (StorableMockDaService, watch::Sender<()>, SocketAddr) {
    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(());
    let mut da_config = sov_mock_da::MockDaConfig::instant_with_sender(TEST_SEQ_DA_ADDRESS);
    da_config.block_producing = BlockProducingConfig::Periodic {
        block_time_ms: TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS * 2,
    };

    let da_service = StorableMockDaService::from_config(da_config, shutdown_receiver).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    (da_service, shutdown_sender, addr)
}

/// Holds resources for a cluster info subscription.
struct ClusterInfoSubscription {
    _temp_dir: tempfile::TempDir,
    path: PathBuf,
    handle: JoinHandle<()>,
    file_watcher: watch::Receiver<()>,
}

impl ClusterInfoSubscription {
    async fn wait_for_change(&mut self) -> ClusterInfo {
        tokio::time::timeout(Duration::from_secs(5), self.file_watcher.changed())
            .await
            .unwrap()
            .unwrap();

        let content = std::fs::read_to_string(&self.path).expect("Failed to read file content");
        ClusterInfo::parse(&content).expect("Failed to parse cluster info")
    }

    fn abort(self) {
        self.handle.abort();
    }
}

/// Test setup for ToxiProxy-based tests with two nodes (leader and replica).
struct ToxiProxyTestSetup {
    toxiproxy_postgres: Arc<ToxiProxyPostgresData>,
    da_addr: SocketAddr,
    da_shutdown: watch::Sender<()>,
    cluster_info_subscription: ClusterInfoSubscription,
}

const MAX_AGE: Duration = Duration::from_secs(10);

impl ToxiProxyTestSetup {
    /// Creates a new test setup with ToxiProxy in front of PostgreSQL.
    /// Returns None if Docker is not supported.
    async fn new() -> Option<Self> {
        let toxiproxy_postgres = match ToxiProxyPostgresData::create().await {
            Ok(data) => data,
            Err(CreatePostgresError::DockerNotSupported) => return None,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create ToxiProxy+Postgres setup: {e}");
            }
        };

        let (_, da_shutdown, da_addr) = create_da_service_periodic().await;

        // Create NodeDiscovery to query the nodes table - use proxied connection
        let (node_discovery, file_watcher) = NodeDiscovery::new_with_max_age(
            toxiproxy_postgres.proxied_connection_string(),
            MAX_AGE,
        )
        .await
        .expect("Failed to create NodeDiscovery");

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("cluster_info.txt");
        let path_clone = path.clone();

        let handle = tokio::spawn(async move {
            node_discovery
                .subscribe_cluster_info_loop(path_clone)
                .await
                .unwrap();
        });

        let cluster_info_subscription = ClusterInfoSubscription {
            _temp_dir: temp_dir,
            path,
            handle,
            file_watcher,
        };

        Some(Self {
            toxiproxy_postgres,
            da_shutdown,
            da_addr,
            cluster_info_subscription,
        })
    }

    /// Start a node with the proxied PostgreSQL connection.
    async fn start_node(&self, node_id: &str, role: ConfiguredNodeRole) -> TestRollup<Rollup> {
        let genesis = test_genesis_source(OperatingMode::Operator);

        RollupBuilder::new_with_external_da_and_connection_string(
            genesis,
            MockDaClientConfig {
                url: format!("http://{}", self.da_addr),
            },
            Some((
                self.toxiproxy_postgres
                    .proxied_connection_string()
                    .to_string(),
                node_id.to_string(),
                role,
            )),
        )
        .set_config(|c| match &mut c.sequencer_config {
            SequencerKindConfig::Standard(_) => {}
            SequencerKindConfig::Preferred(p) => {
                p.num_cache_warmup_workers = 0;
            }
        })
        .start_test_rollup()
        .await
        .unwrap()
    }

    /// Disable the proxy to simulate PostgreSQL connection failure.
    async fn disable_postgres_connection(&self) -> anyhow::Result<()> {
        self.toxiproxy_postgres.disable_proxy().await
    }

    /// Re-enable the proxy to restore PostgreSQL connection.
    async fn enable_postgres_connection(&self) -> anyhow::Result<()> {
        self.toxiproxy_postgres.enable_proxy().await
    }

    /// Add latency to PostgreSQL connections.
    async fn add_latency(&self, latency_ms: u64) -> anyhow::Result<()> {
        self.toxiproxy_postgres.add_latency(latency_ms, 0).await
    }

    /// Remove all network faults.
    async fn reset_network(&self) -> anyhow::Result<()> {
        self.toxiproxy_postgres.reset().await
    }

    async fn shutdown(self) {
        self.cluster_info_subscription.abort();
        let _ = self.da_shutdown.send(());
        // Properly stop containers to avoid "Cannot drop a runtime" panic
        self.toxiproxy_postgres.stop().await;
    }
}

async fn establish_leader_and_replica(
    node_1: TestRollup<Rollup>,
    node_2: TestRollup<Rollup>,
) -> (TestRollup<Rollup>, TestRollup<Rollup>) {
    let role1 = node_1.sequencer_role().await.unwrap();
    let role2 = node_2.sequencer_role().await.unwrap();

    match (role1, role2) {
        (SequencerRole::BatchProducer, SequencerRole::PgSyncReplica) => (node_1, node_2),
        (SequencerRole::PgSyncReplica, SequencerRole::BatchProducer) => (node_2, node_1),
        _ => {
            panic!("Expected one BatchProducer and one PgSyncReplica, got {role1:?} and {role2:?}")
        }
    }
}

/// Test that when PostgreSQL connection is lost, the leader detects the failure
/// and eventually the replica can take over leadership after reconnection.
#[tokio::test(flavor = "multi_thread")]
async fn test_leader_failover_on_postgres_disconnect() {
    let Some(mut setup) = ToxiProxyTestSetup::new().await else {
        return;
    };

    let _key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    // Start two DbElected nodes
    let node_1 = setup
        .start_node("node_leader", ConfiguredNodeRole::DbElected)
        .await;

    let node_2 = setup
        .start_node("node_replica", ConfiguredNodeRole::DbElected)
        .await;

    node_1.wait_for_sequencer_ready().await.unwrap();
    node_2.wait_for_sequencer_ready().await.unwrap();

    let (leader, replica) = establish_leader_and_replica(node_1, node_2).await;

    // Verify initial state via cluster info
    let cluster_info = setup.cluster_info_subscription.wait_for_change().await;
    tracing::info!(
        "Initial cluster state - Leader: {:?}, Followers: {:?}",
        cluster_info.leader,
        cluster_info.followers
    );

    assert!(
        cluster_info.leader.is_some(),
        "Expected a leader to be present"
    );

    // Disable PostgreSQL connection via ToxiProxy
    tracing::info!("Disabling PostgreSQL connection via ToxiProxy...");
    setup.disable_postgres_connection().await.unwrap();

    // Wait for the leader to detect the failure and shut down
    let start = std::time::Instant::now();
    while !leader.is_rollup_crashed() {
        if start.elapsed() > Duration::from_secs(30) {
            // Re-enable connection before failing to clean up properly
            let _ = setup.enable_postgres_connection().await;
            panic!("Timeout waiting for leader to detect PostgreSQL failure and shutdown");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    tracing::info!("Leader detected PostgreSQL failure and shut down");

    // Re-enable PostgreSQL connection
    tracing::info!("Re-enabling PostgreSQL connection...");
    setup.enable_postgres_connection().await.unwrap();

    // Wait for replica to shutdown (it should acquire leadership and call exit_rollup)
    let start = std::time::Instant::now();
    while !replica.is_rollup_crashed() {
        if start.elapsed() > Duration::from_secs(30) {
            panic!("Timeout waiting for replica to acquire leadership and shutdown");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Get the builder from the replica
    let replica_builder = replica.shutdown().await.unwrap();

    // Restart the former replica
    let restarted_rollup = replica_builder.start_test_rollup().await.unwrap();
    restarted_rollup.wait_for_sequencer_ready().await.unwrap();

    // Verify the restarted node is now the leader
    let new_role = restarted_rollup.sequencer_role().await.unwrap();
    assert_eq!(
        new_role,
        SequencerRole::BatchProducer,
        "Expected restarted node to be BatchProducer, got {new_role:?}"
    );

    let _ = restarted_rollup.shutdown().await;
    let _ = leader.shutdown().await;
    setup.shutdown().await;
}

/// Test that a replica survives a brief PostgreSQL disconnect and continues
/// to function after reconnection.
#[tokio::test(flavor = "multi_thread")]
async fn test_replica_survives_brief_postgres_disconnect() {
    let Some(setup) = ToxiProxyTestSetup::new().await else {
        return;
    };

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    // Start two DbElected nodes
    let node_1 = setup
        .start_node("node_leader", ConfiguredNodeRole::DbElected)
        .await;

    let node_2 = setup
        .start_node("node_replica", ConfiguredNodeRole::DbElected)
        .await;

    node_1.wait_for_sequencer_ready().await.unwrap();
    node_2.wait_for_sequencer_ready().await.unwrap();

    let (leader, replica) = establish_leader_and_replica(node_1, node_2).await;

    // Subscribe to events on the replica
    let mut event_subscription = replica
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    // Send initial transaction to verify the system works
    let receiver_addr = random_address();
    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        config_gas_token_id(),
        receiver_addr,
        100,
        0,
    );
    leader.send_tx_to_sequencer(&tx).await.unwrap();

    // Wait for event on replica
    tokio::time::timeout(Duration::from_millis(500), event_subscription.next())
        .await
        .unwrap();

    // Briefly disable PostgreSQL connection
    tracing::info!("Briefly disabling PostgreSQL connection...");
    setup.disable_postgres_connection().await.unwrap();

    // Wait a short time (not long enough to trigger failover)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Re-enable connection
    tracing::info!("Re-enabling PostgreSQL connection...");
    setup.enable_postgres_connection().await.unwrap();

    // Verify both nodes are still operational
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Leader should still be able to accept transactions
    let tx2 = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        config_gas_token_id(),
        receiver_addr,
        100,
        1,
    );

    // Retry sending since the connection may need time to recover
    let mut sent = false;
    for _ in 0..10 {
        match leader.send_tx_to_sequencer(&tx2).await {
            Ok(_) => {
                sent = true;
                break;
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    assert!(sent, "Failed to send transaction after reconnection");

    // Verify replica still receives transactions
    tokio::time::timeout(Duration::from_secs(2), event_subscription.next())
        .await
        .expect("Timeout waiting for event on replica after reconnection");

    replica.shutdown().await.unwrap();
    leader.shutdown().await.unwrap();
    setup.shutdown().await;
}

/// Test that high latency on PostgreSQL connections eventually triggers failover.
#[tokio::test(flavor = "multi_thread")]
async fn test_high_latency_triggers_failover() {
    let Some(mut setup) = ToxiProxyTestSetup::new().await else {
        return;
    };

    println!("X1");

    // Start two DbElected nodes
    let node_1 = setup
        .start_node("node_leader", ConfiguredNodeRole::DbElected)
        .await;

    let node_2 = setup
        .start_node("node_replica", ConfiguredNodeRole::DbElected)
        .await;

    println!("X2");
    node_1.wait_for_sequencer_ready().await.unwrap();
    node_2.wait_for_sequencer_ready().await.unwrap();

    println!("X3");

    let (leader, replica) = establish_leader_and_replica(node_1, node_2).await;

    println!("X4");
    // Verify initial cluster state
    let cluster_info = setup.cluster_info_subscription.wait_for_change().await;
    assert!(
        cluster_info.leader.is_some(),
        "Expected a leader to be present"
    );

    println!("X5");
    // Add very high latency (> leader_timeout which defaults to 500ms)
    tracing::info!("Adding 2000ms latency to PostgreSQL connections...");
    setup.add_latency(2000).await.unwrap();

    println!("X6");
    // Wait for leader to detect timeout and shut down
    let start = std::time::Instant::now();
    while !leader.is_rollup_crashed() {
        if start.elapsed() > Duration::from_secs(60) {
            let _ = setup.reset_network().await;
            panic!("Timeout waiting for leader to fail due to high latency");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    tracing::info!("Leader failed due to high latency");

    println!("X7");
    // Reset network conditions
    setup.reset_network().await.unwrap();

    // Wait for replica to acquire leadership
    let start = std::time::Instant::now();
    while !replica.is_rollup_crashed() {
        if start.elapsed() > Duration::from_secs(30) {
            panic!("Timeout waiting for replica to acquire leadership");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    println!("X8");
    // Restart the replica as new leader
    let replica_builder = replica.shutdown().await.unwrap();
    let new_leader = replica_builder.start_test_rollup().await.unwrap();
    new_leader.wait_for_sequencer_ready().await.unwrap();

    let role = new_leader.sequencer_role().await.unwrap();
    assert_eq!(
        role,
        SequencerRole::BatchProducer,
        "Expected new leader to be BatchProducer"
    );
    println!("X9");

    let _ = new_leader.shutdown().await;
    println!("X10");
    let _ = leader.shutdown().await;
    println!("X11");
    setup.shutdown().await;
}
