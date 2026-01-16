use super::*;

use sov_sequencer::SequencerRole;
use tokio::time::Duration;

/// Test that when two DbElected nodes start, one becomes leader and the other becomes replica.
/// The leader can process transactions while the replica receives them via PostgreSQL sync.
#[tokio::test(flavor = "multi_thread")]
async fn test_db_elected_two_nodes_leader_and_replica() {
    let postgres = PostgresData::create_postgres().await;

    let postgres = match postgres {
        Ok(pg) => Some(pg),
        Err(CreatePostgresError::DockerNotSupported) => return,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let key_and_address = read_private_key::<S>("tx_signer_private_key.json");
    let (_, da_shutdown, addr) = create_da_service_periodic().await;

    // Start both DbElected nodes
    let node1 = postgres
        .clone()
        .map(|pg| (pg, "node1".into(), NodeRole::DbElected));
    let rollup1 = start_rollup(addr, node1).await;

    let node2 = postgres.map(|pg| (pg, "node2".into(), NodeRole::DbElected));
    let rollup2 = start_rollup(addr, node2).await;

    // Wait for both nodes to be ready
    rollup1.wait_for_sequencer_ready().await.unwrap();
    rollup2.wait_for_sequencer_ready().await.unwrap();

    // Discover roles via the /sequencer/role endpoint
    let role1 = rollup1.sequencer_role().await.unwrap();
    let role2 = rollup2.sequencer_role().await.unwrap();

    // Determine which node is the leader and which is the replica
    let (leader_rollup, replica_rollup) = match (role1, role2) {
        (SequencerRole::Leader, SequencerRole::Replica) => (&rollup1, &rollup2),
        (SequencerRole::Replica, SequencerRole::Leader) => (&rollup2, &rollup1),
        _ => panic!(
            "Expected one Leader and one Replica, got {:?} and {:?}",
            role1, role2
        ),
    };

    // Subscribe to events on the replica
    let mut event_subscription = replica_rollup
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

    leader_rollup.send_tx_to_sequencer(&tx).await.unwrap();

    // Wait for the event on the replica (proves it received the tx via PostgreSQL sync)
    wait_for_all_events_with_timeout(Duration::from_millis(500), 1, &mut event_subscription).await;

    let _ = rollup1.shutdown().await;
    let _ = rollup2.shutdown().await;
    let _ = da_shutdown.send(());
}
