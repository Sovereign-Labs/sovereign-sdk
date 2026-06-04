//! Shared helpers for `sov-proxy-utils` integration tests.
//!
//! Each integration test file compiles `common` as part of its own crate, so
//! any helper not used by that particular test would otherwise be flagged as
//! dead code.
#![allow(dead_code)]

use std::time::Duration;

use sov_proxy_utils::{ClusterUpdateNotifier, NodeDiscovery, NodeDiscoveryTask};
use sov_test_utils::postgres::{
    connection_string_from_postgres_container, create_postgres_container, ContainerAsync,
    CreatePostgresError, Postgres,
};
use sov_test_utils::sov_toxi_proxi_image::ToxiProxySetup;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// The sequencer's production migrations.
pub const MIGRATIONS: &[&str] = &[
    include_str!(
        "../../../../full-node/sov-sequencer/src/preferred/db/postgres/migrations/001_init.sql"
    ),
    include_str!(
        "../../../../full-node/sov-sequencer/src/preferred/db/postgres/migrations/002_nodes.sql"
    ),
    include_str!(
        "../../../../full-node/sov-sequencer/src/preferred/db/postgres/migrations/003_unique_tx_events.sql"
    ),
];

/// Spins up a Postgres container with [`MIGRATIONS`] applied.
///
/// Returns `None` when Docker is unavailable so the test can skip cleanly. The
/// returned [`ContainerAsync`] must be kept alive for the whole test: dropping
/// it stops the database.
pub async fn setup() -> Option<(ContainerAsync<Postgres>, String, PgPool)> {
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
pub async fn insert_node(pool: &PgPool, node_id: &str, address: &str) {
    sqlx::query("INSERT INTO nodes (node_id, address, last_updated) VALUES ($1, $2, NOW())")
        .bind(node_id)
        .bind(address)
        .execute(pool)
        .await
        .expect("Failed to insert node");
}

/// Records the given node as the current leader in the `sequencer_leader`
/// table. The node should already exist in `nodes` (see [`insert_node`]),
/// otherwise discovery will report it as a missing leader.
pub async fn set_leader(pool: &PgPool, node_id: &str) {
    sqlx::query(
        "INSERT INTO sequencer_leader (node_id, last_updated) VALUES ($1, NOW()) \
         ON CONFLICT (singleton) DO UPDATE SET node_id = EXCLUDED.node_id, last_updated = EXCLUDED.last_updated",
    )
    .bind(node_id)
    .execute(pool)
    .await
    .expect("Failed to set leader");
}

/// Waits for the discovery task to publish a cluster info update.
pub async fn wait_for_change(task: &mut NodeDiscoveryTask) {
    tokio::time::timeout(Duration::from_secs(20), task.receiver.changed())
        .await
        .expect("Timed out waiting for a cluster info update")
        .expect("Cluster info watch channel closed");
}

/// Spawns a [`NodeDiscovery`] task connected directly to Postgres with the
/// given notifier.
pub async fn start_discovery(
    connection_string: &str,
    poll_interval: Duration,
    notifier: Option<Box<dyn ClusterUpdateNotifier>>,
) -> NodeDiscoveryTask {
    NodeDiscovery::connect(
        connection_string,
        Duration::from_secs(300), // max_age: keep nodes well within the window.
        poll_interval,
        notifier,
    )
    .await
    .expect("Failed to connect NodeDiscovery")
    .spawn()
}

/// Starts toxiproxy in front of Postgres and spawns a [`NodeDiscovery`] task
/// that connects through it.
pub async fn start_proxied_discovery(
    connection_string: &str,
    poll_interval: Duration,
) -> (ToxiProxySetup, NodeDiscoveryTask) {
    let toxiproxy = ToxiProxySetup::start_for_postgres(connection_string).await;
    let task = start_discovery(
        toxiproxy.proxied_postgres_connection_string(),
        poll_interval,
        None,
    )
    .await;
    (toxiproxy, task)
}
