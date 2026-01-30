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
    let cluster_info = setup.cluster_info_subscription.wait_for_change().await;
    assert_eq!(cluster_info.followers.len(), 2);
    assert!(cluster_info.has_follower("replica_then_leader"));
    assert!(cluster_info.has_follower("replica"));

    let mut builder = replica_then_leader.shutdown().await.unwrap();
    // Upgrade to leader.
    builder.set_as_leader();

    let leader = builder.start_test_rollup().await.unwrap();
    leader.wait_for_sequencer_ready().await.unwrap();

    let mut cluster_info = setup.cluster_info_subscription.wait_for_change().await;

    assert!(cluster_info.has_leader("replica_then_leader"));
    assert!(cluster_info.has_follower("replica"));

    // Verify that nodes timestamps are increasing.
    for _ in 0..3 {
        let new_cluster_info = setup.cluster_info_subscription.wait_for_change().await;
        assert!(
            new_cluster_info.leader.as_ref().unwrap().last_updated
                >= cluster_info.leader.as_ref().unwrap().last_updated
        );

        // This is O(n^2), but it's we have only two nodes in this test.
        for new_follower in &new_cluster_info.followers {
            let follower = cluster_info
                .followers
                .iter()
                .find(|f| f.node_id == new_follower.node_id)
                .unwrap();

            assert!(new_follower.last_updated >= follower.last_updated);
        }

        cluster_info = new_cluster_info;
    }

    let _ = leader.shutdown();
    let _ = replica_rollup.shutdown().await;
    setup.shutdown().await;
}
