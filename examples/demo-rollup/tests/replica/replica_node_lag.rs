use super::*;

/// Regression test for `ensure_replica_batch_start_visible_slot_has_node_state`.
///
/// When a replica's node state lags behind the leader, the leader persists a batch-start whose
/// `visible_slot_number_after_increase` is past the replica node's latest slot. The replica must
/// reject that batch-start, surface the `Syncing` not-ready reason, and keep retrying without
/// crashing, then recover once its node catches up.
///
/// We reproduce the lag by pausing only the replica's `update_state` loop (which freezes its
/// `latest_info.slot_number`) while the leader keeps advancing on a healthy Postgres + DA.
#[tokio::test(flavor = "multi_thread")]
async fn test_replica_rejects_batch_start_beyond_node_state_and_recovers() {
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

    // Confirm the cluster is healthy and the replica has finished startup and caught up, so the
    // guard returns `Syncing` (node behind) rather than `Startup`.
    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        0,
    )
    .await;

    // Freeze ONLY the replica's node state by pausing its `update_state` loop. Postgres stays
    // healthy, so the replica keeps pulling the leader's batch-starts from the shared DB.
    replica.pause_preferred_batches_for_node().await;

    // Keep the leader producing real batches while the DA advances, so it persists batch-starts
    // whose `visible_slot_number_after_increase` climbs past the replica's now-frozen node slot,
    // tripping `ensure_replica_batch_start_visible_slot_has_node_state`.
    send_transfers(
        1,
        3,
        read_private_key::<S>("tx_signer_private_key.json"),
        receiver_addr,
        leader.api_client().clone(),
    )
    .await;

    for _ in 0..60 {
        setup.da_service.produce_block_now().await.unwrap();
    }

    // The replica rejects the future-slot batch-start with `Syncing` and keeps retrying without
    // crashing. `wait_for_sequencer_syncing` fails fast if the replica crashes instead.
    replica.wait_for_sequencer_syncing().await.unwrap();
    assert!(
        !replica.is_rollup_crashed(),
        "the replica batch-start guard must reject the batch without crashing the rollup"
    );

    // Resume the replica: its node catches up past the batch's visible slot and it becomes ready.
    leader.wait_for_node_synced().await.unwrap();
    replica.resume_preferred_batches_for_node().await;
    setup.da_service.produce_block_now().await.unwrap();
    replica.wait_for_sequencer_ready().await.unwrap();

    // Confirm the cluster still works end-to-end after recovery. Nonces 1-3 were used by the
    // transfers sent during the replica's downtime above, so the next transfer uses nonce 4.
    verify_replica_processes_tx(
        &leader,
        &replica,
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        4,
    )
    .await;

    replica.shutdown().await.unwrap();
    leader.shutdown().await.unwrap();
    setup.shutdown().await;
}
