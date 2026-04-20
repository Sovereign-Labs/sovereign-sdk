use super::*;

/// Test that if the leader enters recovery after falling behind the deferred-slots threshold,
/// the replica continues to function correctly once recovery completes.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_leader_recovery_with_replica() {
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

    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        0,
    )
    .await;

    // Pause the sequencer update_state loop to prevent batch production.
    leader.pause_preferred_batches().await;

    for _ in 0..30 {
        setup.da_service.produce_block_now().await.unwrap();
    }

    // Wait for the nodes to sync the DA blocks
    leader.wait_for_node_synced().await.unwrap();

    // Resume batch production; on the next state update the leader should enter recovery
    leader.resume_preferred_batches().await;

    // Wait until the leader enters recovery.
    leader.wait_for_sequencer_recovering().await.unwrap();
    leader.wait_for_sequencer_ready().await.unwrap();

    // // Send a transaction to confirm the cluster works after recovery.
    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        1,
    )
    .await;

    replica.shutdown().await.unwrap();
    leader.shutdown().await.unwrap();
    setup.shutdown().await;
}

/// Test that if only the elected leader pauses batch production and enters
/// recovery after falling behind, the replica continues to function correctly
/// once recovery completes.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_leader_recovery_with_replica_when_only_leader_is_paused() {
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

    // Send a transaction to confirm the cluster works before recovery.
    let token_id = config_gas_token_id();
    let receiver_addr = random_address();

    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        0,
    )
    .await;

    // Pause only the elected leader's sequencer update_state loop.
    leader.pause_preferred_batches_for_node().await;

    for _ in 0..30 {
        setup.da_service.produce_block_now().await.unwrap();
    }

    leader.wait_for_node_synced().await.unwrap();

    // Resume batch production only for the leader; on the next state update it should enter recovery.
    leader.resume_preferred_batches_for_node().await;

    // Wait until the leader enters recovery.
    leader.wait_for_sequencer_recovering().await.unwrap();
    leader.wait_for_sequencer_ready().await.unwrap();

    // Send a transaction to confirm the cluster works after recovery.
    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        1,
    )
    .await;

    replica.shutdown().await.unwrap();
    leader.shutdown().await.unwrap();
    setup.shutdown().await;
}

/// Test that if only the replica pauses `update_state`, it enters sync mode
/// while the leader continues to process batches, then catches back up after resuming.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_replica_recovers_after_only_replica_is_paused() {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_STATE_ROOT_DELAY_BLOCKS", "5");
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

    let token_id = config_gas_token_id();
    let receiver_addr = random_address();

    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        0,
    )
    .await;

    replica.pause_preferred_batches_for_node().await;

    for _ in 0..60 {
        setup.da_service.produce_block_now().await.unwrap();
    }

    replica.wait_for_sequencer_not_ready().await.unwrap();
    leader.wait_for_node_synced().await.unwrap();
    replica.wait_for_node_synced().await.unwrap();

    replica.resume_preferred_batches_for_node().await;
    setup.da_service.produce_block_now().await.unwrap();
    replica.wait_for_sequencer_ready().await.unwrap();

    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        1,
    )
    .await;

    replica.shutdown().await.unwrap();
    leader.shutdown().await.unwrap();
    setup.shutdown().await;
}

async fn verify_replica_processes_tx(
    leader: &TestRollup<Rollup>,
    replica: &TestRollup<Rollup>,
    key: &<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    token_id: sov_bank::TokenId,
    receiver_addr: <S as Spec>::Address,
    nonce: u64,
) {
    let tx = build_transfer_token_tx::<S>(key, token_id, receiver_addr, AMOUNT, nonce);

    let mut event_subscription = replica
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();

    leader.send_tx_to_sequencer(&tx).await.unwrap();
    wait_for_all_events_with_timeout(Duration::from_millis(3500), 1, &mut event_subscription).await;
}
