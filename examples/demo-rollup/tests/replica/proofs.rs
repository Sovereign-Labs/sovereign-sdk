use super::*;
use anyhow::Context;
use futures::StreamExt;

/// Test that when the leader dies, the replica acquires leadership and becomes the new leader.
#[tokio::test(flavor = "multi_thread")]
async fn test_failover_and_zk_proof() {
    let Some(mut setup) = NodeDiscoveryTestSetup::new().await else {
        return;
    };

    let node_1 = setup
        .start_node_with_prover("node_1", ConfiguredNodeRole::DbElected)
        .await;

    let node_2 = setup
        .start_node_with_prover("node_2", ConfiguredNodeRole::DbElected)
        .await;

    node_1.wait_for_sequencer_ready().await.unwrap();
    node_2.wait_for_sequencer_ready().await.unwrap();

    let (leader, replica) = establish_leader_and_replica(node_1, node_2).await;
    let cluster_info = setup.wait_for_cluster_change().await;

    let initial_follower = cluster_info
        .followers
        .values()
        .next()
        .expect("Expected exactly one follower in nodes table");

    // Remember the follower's node_id - this should become the new leader after failover.
    let expected_new_leader_id = initial_follower.node_id.clone();

    let mut proof_sub = leader.subscribe_aggregated_proof().await.unwrap();

    for i in 0..5 {
        tokio::time::timeout(Duration::from_secs(20), proof_sub.next())
            .await
            .unwrap()
            .unwrap();
    }

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

    let mut proof_sub = restarted_rollup.subscribe_aggregated_proof().await.unwrap();

    for i in 0..25 {
        tokio::time::timeout(Duration::from_secs(20), proof_sub.next())
            .await
            .unwrap()
            .unwrap();
    }

    let _ = restarted_rollup.shutdown().await;
    let _ = setup.shutdown().await;
}
