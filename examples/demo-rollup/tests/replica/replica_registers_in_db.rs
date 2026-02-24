use super::*;

/// Tests that replica nodes correctly register in the Postgres nodes table.
///
/// This test verifies:
/// 1. Multiple replica nodes can register in the nodes table simultaneously
/// 2. Replicas appear as followers in the cluster info
/// 3. A replica can transition to leader role and be recognized as such after a restart.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_replicas_register_in_nodes_table() {
    let Some(mut setup) = NodeDiscoveryTestSetup::new().await else {
        return;
    };

    let replica_then_leader = setup
        .start_node("replica_then_leader", ConfiguredNodeRole::Replica)
        .await;

    let replica_rollup = setup
        .start_node("replica", ConfiguredNodeRole::Replica)
        .await;

    // Verify both replicas are present in followers
    let cluster_info = setup.wait_for_cluster_change().await;

    assert_eq!(cluster_info.followers.len(), 2);
    assert!(cluster_info.has_follower("replica_then_leader"));
    assert!(cluster_info.has_follower("replica"));

    let old_follower = cluster_info.followers.get("replica").unwrap();

    let mut builder = replica_then_leader.shutdown().await.unwrap();
    // Upgrade to leader.
    builder.set_as_leader();

    let leader = builder.start_test_rollup().await.unwrap();
    leader.wait_for_sequencer_ready().await.unwrap();

    let new_cluster_info = setup.wait_for_cluster_change().await;

    assert!(new_cluster_info.has_leader("replica_then_leader"));
    assert!(new_cluster_info.has_follower("replica"));

    // Verify that nodes timestamps are increasing.
    let new_follower = new_cluster_info.followers.get("replica").unwrap();

    assert!(new_follower.last_updated > old_follower.last_updated);

    let _ = leader.shutdown().await;
    let _ = replica_rollup.shutdown().await;
    setup.shutdown().await;
}

/// Tests that stale nodes are filtered out from cluster info based on max_age.
///
/// This test verifies cluster-info update behavior:
/// 1. Nodes whose `last_updated` timestamp exceeds `max_age` are filtered out
/// 2. The leader is always included regardless of its age
/// 3. Active nodes continue to appear in the cluster info
#[tokio::test(flavor = "multi_thread")]
async fn test_stale_nodes_are_filtered_from_cluster_info() {
    // Use a short max_age (2 seconds) to speed up the test.
    let max_age = Duration::from_secs(2);
    let Some(mut setup) = NodeDiscoveryTestSetup::new_with_max_age(max_age).await else {
        return;
    };

    // Start a leader node that will keep sending heartbeats.
    let leader = setup.start_node("leader", ConfiguredNodeRole::Leader).await;
    leader.wait_for_sequencer_ready().await.unwrap();

    // Start a replica node.
    let replica = setup
        .start_node("replica", ConfiguredNodeRole::Replica)
        .await;

    replica.wait_for_sequencer_ready().await.unwrap();

    // Wait for both nodes to appear in cluster info.
    let cluster_info = setup.wait_for_cluster_change().await;
    assert!(
        cluster_info.has_leader("leader"),
        "Leader should be present"
    );
    assert!(
        cluster_info.has_follower("replica"),
        "Replica should be present as follower"
    );

    // Shut down the replica - it will stop sending heartbeats.
    let _ = replica.shutdown().await;

    // Wait for the replica to become stale and be filtered out.
    let cluster_info = setup
        .wait_for_cluster_change_with_timeout(Duration::from_secs(10))
        .await;

    // Leader should always be present (never filtered regardless of age).
    assert!(
        cluster_info.has_leader("leader"),
        "Leader should always be present in cluster info"
    );

    // With follower storage keyed by follower node IDs, no follower should remain
    // once the replica becomes stale.
    assert!(cluster_info.followers.is_empty());
    assert!(!cluster_info.has_follower("leader"));
    assert!(!cluster_info.has_follower("replica"));

    let _ = leader.shutdown().await;
    setup.shutdown().await;
}
