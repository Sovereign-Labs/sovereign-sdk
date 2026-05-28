//! Integration tests for [`NodeDiscovery`] against a real PostgreSQL instance.

use std::time::Duration;

use sov_proxy_utils::{NodeDiscovery, NodeDiscoveryTask};
use sov_test_utils::postgres::{
    connection_string_from_postgres_container, create_postgres_container, ContainerAsync,
    CreatePostgresError, Postgres,
};
use sov_test_utils::sov_toxi_proxi_image::ToxiProxySetup;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// The sequencer's production migrations.
const MIGRATIONS: &[&str] = &[
    include_str!(
        "../../../full-node/sov-sequencer/src/preferred/db/postgres/migrations/001_init.sql"
    ),
    include_str!(
        "../../../full-node/sov-sequencer/src/preferred/db/postgres/migrations/002_nodes.sql"
    ),
    include_str!(
        "../../../full-node/sov-sequencer/src/preferred/db/postgres/migrations/003_unique_tx_events.sql"
    ),
];

/// Spins up a Postgres container with [`MIGRATIONS`] applied.
///
/// Returns `None` when Docker is unavailable so the test can skip cleanly. The
/// returned [`ContainerAsync`] must be kept alive for the whole test: dropping
/// it stops the database.
async fn setup() -> Option<(ContainerAsync<Postgres>, String, PgPool)> {
    let container = match create_postgres_container().await {
        Ok(container) => container,
        Err(CreatePostgresError::DockerNotSupported) => return None,
        Err(CreatePostgresError::DockerError(e)) => {
            panic!("Failed to create Postgres container: {e}");
        }
    };

    let connection_string = connection_string_from_postgres_container(&container)
        .await
        .expect("Failed to build connection string");

    let writer = PgPoolOptions::new()
        .connect(&connection_string)
        .await
        .expect("Failed to connect writer pool");

    for migration in MIGRATIONS {
        sqlx::raw_sql(migration)
            .execute(&writer)
            .await
            .expect("Failed to apply migration");
    }

    Some((container, connection_string, writer))
}

/// Inserts a node into the `nodes` table with a fresh heartbeat.
async fn insert_node(pool: &PgPool, node_id: &str, address: &str) {
    sqlx::query("INSERT INTO nodes (node_id, address, last_updated) VALUES ($1, $2, NOW())")
        .bind(node_id)
        .bind(address)
        .execute(pool)
        .await
        .expect("Failed to insert node");
}

/// Waits for the discovery task to publish a cluster info update.
async fn wait_for_change(task: &mut NodeDiscoveryTask) {
    tokio::time::timeout(Duration::from_secs(20), task.receiver.changed())
        .await
        .expect("Timed out waiting for a cluster info update")
        .expect("Cluster info watch channel closed");
}

/// Starts toxiproxy in front of Postgres and spawns a [`NodeDiscovery`] task that connects through it.
async fn start_proxied_discovery(
    connection_string: &str,
    poll_interval: Duration,
) -> (ToxiProxySetup, NodeDiscoveryTask) {
    let toxiproxy = ToxiProxySetup::start_for_postgres(connection_string).await;

    let task = NodeDiscovery::connect(
        toxiproxy.proxied_postgres_connection_string(),
        Duration::from_secs(300), // max_age: keep nodes well within the window.
        poll_interval,
        None,
    )
    .await
    .expect("Failed to connect NodeDiscovery")
    .spawn();

    (toxiproxy, task)
}

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
