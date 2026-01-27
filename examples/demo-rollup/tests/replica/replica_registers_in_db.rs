use super::*;
use sov_proxy_utils::{ClusterInfo, NodeDiscovery};

type Rollup = ExternalMockDemoRollup<Native>;

/// Test setup for replica registration tests.
/// Handles common setup: postgres, DA service, leader node, and node discovery.
struct ReplicaRegistrationTestSetup {
    postgres: Arc<PostgresData>,
    da_addr: SocketAddr,
    node_discovery: NodeDiscovery,
    da_shutdown: watch::Sender<()>,
    // Keep file_watcher alive to prevent channel closure
    _file_watcher: watch::Receiver<()>,
}

impl ReplicaRegistrationTestSetup {
    /// Creates a new test setup with postgres, DA service, and a leader node.
    /// Returns None if Docker is not supported.
    async fn new() -> Option<Self> {
        let postgres = match PostgresData::create_postgres().await {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return None,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let (_, da_shutdown, da_addr) = create_da_service_periodic().await;

        // Create NodeDiscovery to query the nodes table
        let (node_discovery, _file_watcher) = NodeDiscovery::new(postgres.connection_string())
            .await
            .expect("Failed to create NodeDiscovery");

        Some(Self {
            postgres,
            da_addr,
            node_discovery,
            da_shutdown,
            _file_watcher,
        })
    }

    /// Starts a replica node with the given node_id and role.
    async fn start_replica(&self, node_id: &str, role: ConfiguredNodeRole) -> TestRollup<Rollup> {
        let replica = Some((self.postgres.clone(), node_id.into(), role));
        let replica_rollup = start_rollup(self.da_addr, replica).await;
        replica_rollup
    }

    /// Gets cluster info from the nodes table.
    async fn get_cluster_info(&self) -> ClusterInfo {
        self.node_discovery
            .get_cluster_info()
            .await
            .expect("Failed to get cluster info")
    }

    /// Gets follower node IDs from the cluster info.
    async fn get_follower_ids(&self) -> Vec<String> {
        self.get_cluster_info()
            .await
            .followers
            .iter()
            .map(|f| f.node_id.clone())
            .collect()
    }

    /// Shuts down the leader and DA service.
    async fn shutdown(self) {
        let _ = self.da_shutdown.send(());
    }
}

/// Tests that replica nodes correctly register in the Postgres nodes table.
///
/// This test verifies:
/// 1. Multiple replica nodes can register in the nodes table simultaneously
/// 2. Replicas appear as followers in the cluster info
/// 3. A replica can transition to leader role and be recognized as such after a restart.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_replicas_register_in_nodes_table() {
    let Some(setup) = ReplicaRegistrationTestSetup::new().await else {
        return;
    };

    let replica_then_leader = setup
        .start_replica("replica_then_leader", ConfiguredNodeRole::Replica)
        .await;

    let replica_rollup = setup
        .start_replica("replica", ConfiguredNodeRole::Replica)
        .await;

    // Verify both replicas are present in followers
    let follower_ids = setup.get_follower_ids().await;
    assert_eq!(follower_ids.len(), 2);
    assert!(follower_ids.contains(&"replica_then_leader".to_string()));
    assert!(follower_ids.contains(&"replica".to_string()));

    let mut builder = replica_then_leader.shutdown().await.unwrap();
    // Upgrade to leader.
    builder.set_as_leader();

    let leader = builder.start_test_rollup().await.unwrap();
    leader.wait_for_sequencer_ready().await.unwrap();

    let cluster_info = setup.get_cluster_info().await;
    let follower_ids = setup.get_follower_ids().await;

    assert_eq!(cluster_info.leader.unwrap().node_id, "replica_then_leader");
    assert!(follower_ids.contains(&"replica".to_string()));

    let _ = replica_rollup.shutdown().await;
    setup.shutdown().await;
}
