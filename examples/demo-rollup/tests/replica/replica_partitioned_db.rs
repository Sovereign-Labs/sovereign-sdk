use super::toxi_proxy_helper::NodeTestSetup;
use super::*;

const BASELINE_TRANSFER_COUNT: u64 = 1;
const PARTITIONED_TRANSFER_COUNT: u64 = 3;

/// Verifies a DB-elected replica remain consistent after sequencer db partition.
#[tokio::test(flavor = "multi_thread")]
async fn test_replica_catches_up_via_da_after_postgres_partition() {
    let Some(mut setup) = NodeTestSetup::new().await else {
        return;
    };

    let (leader, replica) = setup
        .start_db_elected_pair("direct_node", "proxied_node")
        .await;

    let token_id = config_gas_token_id();
    let receiver_addr = random_address();
    let leader_client = leader.api_client().clone();
    let mut leader_bank_events = leader
        .api_client()
        .subscribe_to_events_with_filter("Bank/*")
        .await
        .unwrap();
    let expected_baseline_balance = AMOUNT * u128::from(BASELINE_TRANSFER_COUNT);
    let expected_balance_after_partitioned_tx =
        expected_baseline_balance + AMOUNT * u128::from(PARTITIONED_TRANSFER_COUNT);

    // Both the leader and replica are connected to the same healthy sequencer DB.
    {
        send_transfers(
            0,
            BASELINE_TRANSFER_COUNT,
            read_private_key::<S>("tx_signer_private_key.json"),
            receiver_addr,
            leader_client.clone(),
        )
        .await;

        wait_for_balance(
            &leader,
            &mut leader_bank_events,
            &receiver_addr,
            &token_id,
            expected_baseline_balance,
            BASELINE_TRANSFER_COUNT,
        )
        .await;

        wait_for_replica_to_catchup(&leader, &replica).await;
        let replica_balance = get_balance(&replica, &receiver_addr, &token_id).await;
        assert_eq!(replica_balance, expected_baseline_balance);
    }

    // Simulate a partitioned sequencer DB. Now replica is disconnected and the DA is very slow.
    // `setup.set_replica_da_slow`` ensures that the replica is not updated immediately via DA after a DB partition, allowing the system to diverge for a couple of blocks.
    {
        setup.set_postgres_partition(true).await;
        setup.set_replica_da_slow(true).await;

        send_transfers(
            BASELINE_TRANSFER_COUNT,
            PARTITIONED_TRANSFER_COUNT,
            read_private_key::<S>("tx_signer_private_key.json"),
            receiver_addr,
            leader_client,
        )
        .await;

        wait_for_balance(
            &leader,
            &mut leader_bank_events,
            &receiver_addr,
            &token_id,
            expected_balance_after_partitioned_tx,
            PARTITIONED_TRANSFER_COUNT,
        )
        .await;

        leader.wait_for_rollup_height_advance_by(3).await;
        setup.set_replica_da_slow(false).await;
    }

    // The leader posts blobs to the DA (which resumed normal operation), but the replica is still cut off from the sequencer DB.
    // Check that the replica receives the data via DA.
    {
        wait_for_replica_to_catchup(&leader, &replica).await;
        let replica_balance = get_balance(&replica, &receiver_addr, &token_id).await;
        assert_eq!(replica_balance, expected_balance_after_partitioned_tx);
    }

    // Here the DB partition is ended. We check that we can still query the replica
    setup.set_postgres_partition(false).await;

    let replica_balance = get_balance(&replica, &receiver_addr, &token_id).await;
    assert_eq!(replica_balance, expected_balance_after_partitioned_tx);

    setup.shutdown(leader, replica).await;
}

async fn wait_for_replica_to_catchup(leader: &TestRollup<Rollup>, replica: &TestRollup<Rollup>) {
    let height = leader.height().await.get();
    replica.wait_for_height(height).await;
}

async fn get_balance(
    node: &TestRollup<Rollup>,
    receiver_addr: &<S as Spec>::Address,
    token_id: &sov_bank::TokenId,
) -> u128 {
    node.client
        .get_balance::<S>(receiver_addr, token_id, None)
        .await
        .unwrap()
        .0
}

async fn wait_for_balance(
    node: &TestRollup<Rollup>,
    bank_events: &mut BoxStream<'static, anyhow::Result<types::LedgerEvent>>,
    receiver_addr: &<S as Spec>::Address,
    token_id: &sov_bank::TokenId,
    expected_balance: u128,
    tx_count: u64,
) {
    for _ in 0..tx_count {
        let _ = bank_events
            .next()
            .await
            .expect("Bank event stream unexpectedly closed")
            .expect("Failed to read bank event from subscription");
    }

    let current_balance = get_balance(node, receiver_addr, token_id).await;
    assert_eq!(current_balance, expected_balance);
}
