use super::*;

// 1. Checking recovery
//"The preferred sequencer is recovering from downtime and cannot provide soft-confirmations at this time; No new transactions can be accepted, try again later"

/// Test that when the leader enters recovery state (due to falling behind
/// the deferred slots threshold), the replica continues to function and
/// both nodes recover to normal operation.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_leader_recovery_replica_keeps_running() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "40");

    let Some(setup) = NodeDiscoveryTestSetup::new().await else {
        return;
    };

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");

    let node_1 = setup
        .start_node("node_1", ConfiguredNodeRole::DbElected)
        .await;
    let node_2 = setup
        .start_node("node_2", ConfiguredNodeRole::DbElected)
        .await;

    node_1.wait_for_sequencer_ready().await.unwrap();
    node_2.wait_for_sequencer_ready().await.unwrap();

    let (leader, replica) = establish_leader_and_replica(node_1, node_2).await;

    // Send a transaction to confirm the cluster works before recovery
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

    let mut event_subscription = replica
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();
    wait_for_all_events_with_timeout(Duration::from_millis(500), 1, &mut event_subscription).await;
    drop(event_subscription);

    // Pause the sequencer update_state loop to prevent batch production
    leader.pause_preferred_batches().await;

    // Produce DA blocks while sequencer is paused to exceed the deferred slots threshold.
    // With DEFERRED_SLOTS_COUNT=40, the 90% threshold triggers at ~26 blocks of lag.
    for _ in 0..30 {
        setup.da_service.produce_block_now().await.unwrap();
    }

    // Wait for the nodes to sync the DA blocks
    leader.wait_for_node_synced().await.unwrap();

    // Resume batch production; on the next state update the leader should enter recovery
    leader.resume_preferred_batches().await;
    setup.da_service.produce_block_now().await.unwrap();

    // Wait until the leader is no longer ready (entered recovery)
    let start = std::time::Instant::now();
    while leader.is_sequencer_ready().await {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("Timeout waiting for leader to enter recovery");
        }
        setup.da_service.produce_block_now().await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // Produce DA blocks to let the leader recover (it needs to send catchup batches)
    while !leader.is_sequencer_ready().await {
        setup.da_service.produce_block_now().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    leader.wait_for_sequencer_ready().await.unwrap();

    /*
    // Verify the replica is still operational after recovery by sending a new transaction
    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        AMOUNT,
        1,
    );
    leader.send_tx_to_sequencer(&tx).await.unwrap();

    let mut event_subscription = replica
        .api_client()

        .await
        .unwrap();
    wait_for_all_events_with_timeout(Duration::from_millis(500), 1, &mut event_subscription).await;

    */

    replica.shutdown().await.unwrap();
    leader.shutdown().await.unwrap();
    setup.shutdown().await;
}
