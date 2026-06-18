//! Integration tests for [`NodeDiscovery`] against a real PostgreSQL instance.

use std::time::Duration;

mod common;
use common::{insert_node, setup, start_proxied_discovery, wait_for_change};

/// A membership change made while no notification reaches `NodeDiscovery` is
/// still discovered, because it re-polls cluster info every `poll_interval`.
#[tokio::test(flavor = "multi_thread")]
async fn discovers_membership_change_without_notification() {
    let Some((_container, connection_string, writer)) = setup().await else {
        return; // Docker unavailable — skip.
    };

    // poll_interval is short, to keep the test fast.
    let (toxiproxy, mut task) =
        start_proxied_discovery(&connection_string, Duration::from_millis(500)).await;

    // Baseline: a node inserted while the connection is healthy is discovered via the NOTIFY trigger.
    insert_node(&writer, "node_a", "127.0.0.1:9001").await;
    wait_for_change(&mut task).await;
    assert!(
        task.receiver.borrow_and_update().has_follower("node_a"),
        "node inserted with a healthy connection should be discovered"
    );

    // Sever NodeDiscovery's connection, change membership while it is severed so
    // the NOTIFY never reaches its listener, then heal the connection. The lost
    // notification cannot be replayed, so only the periodic poll can surface
    // this change.
    toxiproxy.set_postgres_partition(true).await;
    insert_node(&writer, "node_b", "127.0.0.1:9002").await;
    toxiproxy.set_postgres_partition(false).await;
    wait_for_change(&mut task).await;
    assert!(
        task.receiver.borrow_and_update().has_follower("node_b"),
        "node added without a notification should be discovered via the periodic poll"
    );

    task.abort();
    toxiproxy.shutdown();
}

/// The discovery task survives losing its database connection: it reconnects
/// and keeps reporting cluster changes instead of exiting.
#[tokio::test(flavor = "multi_thread")]
async fn survives_database_connection_loss() {
    let Some((_container, connection_string, writer)) = setup().await else {
        return; // Docker unavailable — skip.
    };

    // poll_interval is effectively disabled, so recovery is exercised purely by
    // the reconnect path rather than the periodic poll.
    let (toxiproxy, mut task) =
        start_proxied_discovery(&connection_string, Duration::from_secs(100000)).await;

    insert_node(&writer, "node_a", "127.0.0.1:9001").await;
    wait_for_change(&mut task).await;

    // Sever the connection, the same as an abrupt network outage.
    toxiproxy.set_postgres_partition(true).await;
    toxiproxy.set_postgres_partition(false).await;

    // The task must reconnect rather than exit, and keep discovering nodes.
    insert_node(&writer, "node_b", "127.0.0.1:9002").await;
    wait_for_change(&mut task).await;
    assert!(
        task.receiver.borrow_and_update().has_follower("node_b"),
        "discovery task should recover from connection loss and keep discovering nodes"
    );

    task.abort();
    toxiproxy.shutdown();
}
