use super::*;
use sov_proxy_utils::NodeDiscovery;
use sov_sequencer::SequencerRole;
use std::path::Path;
use tokio::time::Duration;

type Rollup = ExternalMockDemoRollup<Native>;

/// Test setup for DbElected tests with two nodes (leader and replica).
struct DbElectedTestSetup {
    postgres: Arc<PostgresData>,
    leader: TestRollup<Rollup>,
    replica: TestRollup<Rollup>,
    da_shutdown: watch::Sender<()>,
}

impl DbElectedTestSetup {
    /// Creates a new test setup with two DbElected nodes.
    /// Returns None if Docker is not supported.
    async fn new() -> Option<Self> {
        let postgres = match PostgresData::create_postgres().await {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return None,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let (_, da_shutdown, addr) = create_da_service_periodic().await;

        // Start both DbElected nodes
        let node1 = Some((postgres.clone(), "node1".into(), NodeRole::DbElected));
        let rollup1 = start_rollup(addr, node1).await;

        let node2 = Some((postgres.clone(), "node2".into(), NodeRole::DbElected));
        let rollup2 = start_rollup(addr, node2).await;

        // Wait for both nodes to be ready
        rollup1.wait_for_sequencer_ready().await.unwrap();
        rollup2.wait_for_sequencer_ready().await.unwrap();

        // Discover roles via the /sequencer/role endpoint
        let role1 = rollup1.sequencer_role().await.unwrap();
        let role2 = rollup2.sequencer_role().await.unwrap();

        // Determine which node is the leader and which is the replica
        let (leader, replica) = match (role1, role2) {
            (SequencerRole::BatchProducer, SequencerRole::PgSyncReplica) => (rollup1, rollup2),
            (SequencerRole::PgSyncReplica, SequencerRole::BatchProducer) => (rollup2, rollup1),
            _ => panic!(
                "Expected one BatchProducer and one PgSyncReplica, got {role1:?} and {role2:?}"
            ),
        };

        Some(Self {
            postgres,
            leader,
            replica,
            da_shutdown,
        })
    }

    async fn shutdown(self) {
        let _ = self.leader.shutdown().await;
        let _ = self.replica.shutdown().await;
        let _ = self.da_shutdown.send(());
    }
}

/// Test that when two DbElected nodes start, one becomes leader and the other becomes replica.
/// The leader can process transactions while the replica receives them via PostgreSQL sync.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_two_nodes_leader_and_replica() {
    let Some(setup) = DbElectedTestSetup::new().await else {
        return;
    };

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    // Subscribe to events on the replica
    let mut event_subscription = setup
        .replica
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    // Send transaction to the leader
    let token_id = config_gas_token_id();
    let receiver_addr = random_address();

    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        AMOUNT,
        0,
    );

    setup.leader.send_tx_to_sequencer(&tx).await.unwrap();

    // Wait for the event on the replica (proves it received the tx via PostgreSQL sync)
    wait_for_all_events_with_timeout(Duration::from_millis(500), 1, &mut event_subscription).await;

    setup.shutdown().await;
}

/// Test that when the leader dies, the replica acquires leadership and becomes the new leader.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_leader_failover() {
    let Some(setup) = DbElectedTestSetup::new().await else {
        return;
    };

    let DbElectedTestSetup {
        postgres,
        leader,
        replica,
        da_shutdown,
    } = setup;

    let (node_discovery, _file_watcher) = NodeDiscovery::new(postgres.connection_string())
        .await
        .expect("Failed to create proxy");

    let cluster_info = node_discovery
        .get_cluster_info()
        .await
        .expect("Failed to get cluster info");

    tracing::info!(
        "Initial cluster state - BatchProducer: {:?}, Followers: {:?}",
        cluster_info.leader,
        cluster_info.followers
    );

    // Verify we have a leader in the nodes table
    let initial_leader = cluster_info
        .leader
        .as_ref()
        .expect("Expected a leader to be present in nodes table");

    // Verify we have exactly one follower in the nodes table
    assert_eq!(
        cluster_info.followers.len(),
        1,
        "Expected exactly one follower in nodes table"
    );

    let initial_follower = &cluster_info.followers[0];

    assert_ne!(
        initial_leader.node_id, initial_follower.node_id,
        "BatchProducer and follower should have different node_ids"
    );

    assert_ne!(
        initial_leader.address, initial_follower.address,
        "BatchProducer and follower should have different addresses"
    );

    // Remember the follower's node_id - this should become the new leader after failover
    let expected_new_leader = initial_follower.clone();

    // Kill the leader
    let _ = leader.shutdown().await;

    // Wait for replica to shutdown (it will acquire leadership and call exit_rollup)
    let timeout_duration = Duration::from_secs(15);
    let start = std::time::Instant::now();

    while !replica.is_rollup_crashed() {
        if start.elapsed() > timeout_duration {
            panic!("Timeout waiting for replica to acquire leadership and shutdown");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Get the builder from the replica (task already finished due to exit_rollup)
    let replica_builder = replica.shutdown().await.unwrap();

    // Restart the former replica
    let restarted_rollup = replica_builder.start_test_rollup().await.unwrap();
    restarted_rollup.wait_for_sequencer_ready().await.unwrap();

    // Verify the restarted node is now the leader
    let new_role = restarted_rollup.sequencer_role().await.unwrap();

    assert_eq!(
        new_role,
        SequencerRole::BatchProducer,
        "Expected restarted node to be BatchProducer, got {new_role:?}",
    );

    // Check final cluster state after failover
    let cluster_info = node_discovery
        .get_cluster_info()
        .await
        .expect("Failed to get cluster info after failover");

    // Verify the former follower is now the leader in the sequencer_leader table
    let new_leader = cluster_info
        .leader
        .as_ref()
        .expect("Expected a leader after failover");

    assert_eq!(new_leader.node_id, expected_new_leader.node_id);

    let _ = restarted_rollup.shutdown().await;
    let _ = da_shutdown.send(());
}

/// Test that `subscribe_cluster_info_loop` receives PostgreSQL notifications
/// and writes updated cluster info to file when nodes register and leadership changes.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscribe_cluster_info_receives_notifications() {
    let postgres = match PostgresData::create_postgres().await {
        Ok(pg) => pg,
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    // Create NodeDiscovery and spawn subscribe_cluster_info_loop
    let (node_discovery, mut file_watcher) = NodeDiscovery::new(postgres.connection_string())
        .await
        .expect("Failed to create NodeDiscovery");

    // Create temp file for cluster info output
    let temp_dir = tempfile::tempdir().unwrap();
    let cluster_info_path = temp_dir.path().join("cluster_info.txt");
    let cluster_info_path_clone = cluster_info_path.clone();
    let subscription_handle = tokio::spawn(async move {
        node_discovery
            .subscribe_cluster_info_loop(cluster_info_path_clone)
            .await
            .unwrap();
    });

    let (_, da_shutdown, addr) = create_da_service_periodic().await;

    // Start first node - it will become leader and trigger notifications
    let node1 = Some((postgres.clone(), "node1".into(), NodeRole::DbElected));
    let rollup1 = start_rollup(addr, node1).await;
    rollup1.wait_for_sequencer_ready().await.unwrap();

    // Wait for file to be updated with leader info.
    let file_content1 = wait_for_file_change(&cluster_info_path, &mut file_watcher).await;

    // Start second node - it will become follower and trigger another notification
    let node2 = Some((postgres.clone(), "node2".into(), NodeRole::DbElected));
    let rollup2 = start_rollup(addr, node2).await;
    rollup2.wait_for_sequencer_ready().await.unwrap();

    // Wait for file to be updated with follower info.
    let file_content2 = wait_for_file_change(&cluster_info_path, &mut file_watcher).await;

    // After replica joined, the leader didn't change.
    assert_eq!(
        file_content1.leader.as_ref().unwrap(),
        file_content2.leader.as_ref().unwrap()
    );

    // The followers are updated.
    assert!(file_content1.followers[0] != file_content2.followers[0]);

    // Cleanup
    subscription_handle.abort();
    let _ = rollup1.shutdown().await;
    let _ = rollup2.shutdown().await;
    let _ = da_shutdown.send(());
}

async fn wait_for_file_change(path: &Path, file_watcher: &mut watch::Receiver<()>) -> FileContent {
    tokio::time::timeout(Duration::from_secs(2), file_watcher.changed())
        .await
        .unwrap()
        .unwrap();

    let content = std::fs::read_to_string(path).expect("Failed to read file content");
    FileContent::parse(&content)
}

struct FileContent {
    leader: Option<String>,
    followers: Vec<String>,
}

impl FileContent {
    fn parse(content: &str) -> FileContent {
        let mut leader = None;
        let mut followers = Vec::new();

        for line in content.lines() {
            if let Some(addr) = line.strip_prefix("leader=") {
                leader = Some(addr.parse().expect("Invalid leader address"));
            } else if let Some(addr) = line.strip_prefix("follower=") {
                followers.push(addr.parse().expect("Invalid follower address"));
            }
        }
        FileContent { leader, followers }
    }
}
