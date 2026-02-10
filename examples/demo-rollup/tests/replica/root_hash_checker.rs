use super::*;

/// Tests that the cluster root hash checker emits matching hashes for two healthy nodes.
#[tokio::test(flavor = "multi_thread")]
async fn test_root_hash_checker_reports_consistent_hashes() {
    let Some(mut setup) = NodeDiscoveryTestSetup::new().await else {
        return;
    };

    let node_1 = setup
        .start_node("node_1", ConfiguredNodeRole::DbElected)
        .await;

    let node_2 = setup
        .start_node("node_2", ConfiguredNodeRole::DbElected)
        .await;

    node_1.wait_for_sequencer_ready().await.unwrap();
    node_2.wait_for_sequencer_ready().await.unwrap();

    let cluster_info = setup
        .wait_for_cluster_change_with_timeout(Duration::from_secs(10))
        .await;

    assert!(cluster_info.leader.is_some(), "Expected a leader");
    assert_eq!(cluster_info.followers.len(), 1, "Expected one follower");

    let (leader, replica) = establish_leader_and_replica(node_1, node_2).await;
    let leader_client = leader.api_client().clone();

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");
    let receiver = random_address();

    // Keep producing transactions to affect the root hash in some nontrivial way.
    let traffic_handle = tokio::spawn(send_transfers_forever(
        leader_client,
        key_and_address,
        receiver,
    ));

    // Wait for a few slots to pass to generate some root hashes.
    leader.wait_for_height(15).await;
    assert_consistent_root_hashes(&mut setup, 3).await;

    traffic_handle.abort();
    let _ = traffic_handle.await;

    let _ = leader.shutdown().await;
    let _ = replica.shutdown().await;
    setup.shutdown().await;
}

async fn send_transfers_forever(
    client: sov_api_spec::client::Client,
    key_and_address: PrivateKeyAndAddress<S>,
    receiver: <S as Spec>::Address,
) {
    let mut nonce: u64 = 0;
    loop {
        send_transfers(nonce, 1, key_and_address.clone(), receiver, client.clone()).await;
        nonce += 1;
    }
}

async fn assert_consistent_root_hashes(setup: &mut NodeDiscoveryTestSetup, checks: usize) {
    for _ in 0..checks {
        let root_hashes = setup
            .wait_for_root_hash_check_with_timeout(Duration::from_secs(10))
            .await;
        assert_eq!(
            root_hashes.check_consistency(),
            RootHashConsistency::AllMatch,
            "Expected matching root hashes, got: {root_hashes:?}",
        );
    }
}
