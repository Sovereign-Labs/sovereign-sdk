use super::*;
use sov_sequencer::SequencerRole;

type Rollup = ExternalMockDemoRollup<Native>;

/// Test that when two DbElected nodes start, one becomes leader and the other becomes replica.
/// The leader can process transactions while the replica receives them via PostgreSQL sync.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_two_nodes_leader_and_replica() {
    let Some(setup) = NodeDiscoveryTestSetup::new().await else {
        return;
    };

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    let node_1 = setup
        .start_node("replica_then_leader", ConfiguredNodeRole::DbElected)
        .await;

    let node_2 = setup
        .start_node("replica", ConfiguredNodeRole::DbElected)
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

    leader.send_tx_to_sequencer(&tx).await.unwrap();

    // Wait for the event on the replica (proves it received the tx via PostgreSQL sync)
    wait_for_all_events_with_timeout(Duration::from_millis(500), 1, &mut event_subscription).await;

    replica.shutdown().await.unwrap();
    leader.shutdown().await.unwrap();
    setup.shutdown().await;
}

/// Test that when the leader dies, the replica acquires leadership and becomes the new leader.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_leader_failover() {
    let Some(mut setup) = NodeDiscoveryTestSetup::new().await else {
        return;
    };

    let node_1 = setup
        .start_node("replica_then_leader", ConfiguredNodeRole::DbElected)
        .await;

    let node_2 = setup
        .start_node("replica", ConfiguredNodeRole::DbElected)
        .await;

    node_1.wait_for_sequencer_ready().await.unwrap();
    node_2.wait_for_sequencer_ready().await.unwrap();

    let (leader, replica) = establish_leader_and_replica(node_1, node_2).await;
    let cluster_info = setup.wait_for_cluster_change().await;

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

    let initial_follower = cluster_info
        .followers
        .values()
        .next()
        .expect("Expected exactly one follower in nodes table");

    assert_ne!(
        initial_leader.node_id, initial_follower.node_id,
        "BatchProducer and follower should have different node_ids"
    );

    assert_ne!(
        initial_leader.address, initial_follower.address,
        "BatchProducer and follower should have different addresses"
    );

    // Remember the follower's node_id - this should become the new leader after failover.
    let expected_new_leader_id = initial_follower.node_id.clone();

    // Kill the leader
    let _ = leader.shutdown().await;

    // Wait for replica to shutdown (it will acquire leadership and call exit_rollup)
    let start = std::time::Instant::now();
    while !replica.is_rollup_crashed() {
        if start.elapsed() > Duration::from_secs(15) {
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
    let cluster_info = setup.wait_for_cluster_change().await;

    // Verify the former follower is now the leader in the sequencer_leader table
    let new_leader = cluster_info
        .leader
        .as_ref()
        .expect("Expected a leader after failover");

    assert_eq!(new_leader.node_id, expected_new_leader_id);

    let _ = restarted_rollup.shutdown().await;
    let _ = setup.shutdown().await;
}

/// Test that `NodeDiscovery` receives PostgreSQL notifications
/// and writes updated cluster info to file when nodes register and leadership changes.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscribe_cluster_info_receives_notifications() {
    let Some(mut setup) = NodeDiscoveryTestSetup::new().await else {
        return;
    };

    // Start first node - it will become leader and trigger notifications
    let node_1 = setup
        .start_node("node1", ConfiguredNodeRole::DbElected)
        .await;
    node_1.wait_for_sequencer_ready().await.unwrap();

    // Wait for file to be updated with leader info.
    let cluster_info_1 = setup.wait_for_cluster_change().await;
    assert!(cluster_info_1.followers.is_empty());

    // Start second node - it will become follower and trigger another notification
    let node_2 = setup
        .start_node("node2", ConfiguredNodeRole::DbElected)
        .await;
    node_2.wait_for_sequencer_ready().await.unwrap();

    // Wait for file to be updated with follower info.
    let cluster_info_2 = setup.wait_for_cluster_change().await;

    // After replica joined, the leader didn't change (compare addresses since timestamps may differ).
    assert_eq!(
        cluster_info_1.leader.as_ref().unwrap().address,
        cluster_info_2.leader.as_ref().unwrap().address
    );

    // `node2` joined as the only follower.
    assert_eq!(
        cluster_info_2.followers.first_key_value().unwrap().0,
        "node2"
    );

    // Cleanup
    let _ = node_1.shutdown().await;
    let _ = node_2.shutdown().await;
    setup.shutdown().await;
}

async fn establish_leader_and_replica(
    node_1: TestRollup<Rollup>,
    node_2: TestRollup<Rollup>,
) -> (TestRollup<Rollup>, TestRollup<Rollup>) {
    // Discover roles via the /sequencer/role endpoint
    let role1 = node_1.sequencer_role().await.unwrap();
    let role2 = node_2.sequencer_role().await.unwrap();

    // Determine which node is the leader and which is the replica
    let (leader, replica) = match (role1, role2) {
        (SequencerRole::BatchProducer, SequencerRole::PgSyncReplica) => (node_1, node_2),
        (SequencerRole::PgSyncReplica, SequencerRole::BatchProducer) => (node_2, node_1),
        _ => {
            panic!("Expected one BatchProducer and one PgSyncReplica, got {role1:?} and {role2:?}")
        }
    };

    (leader, replica)
}
