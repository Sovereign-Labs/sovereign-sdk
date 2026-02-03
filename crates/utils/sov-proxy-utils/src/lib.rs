//! Proxy utilities for querying node information from the database.
//!
//! This crate provides the [`Proxy`] struct to retrieve leader and follower
//! IP addresses from the PostgreSQL database atomically.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::{PgListener, PgPool};
use sqlx::FromRow;
pub use time::OffsetDateTime;
use tokio::sync::watch;

const MAX_DB_ERRORS_ALLOWED: u32 = 10;

const MAX_AGE: Duration = Duration::from_secs(10);

/// Information about a registered node.
#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct NodeInfo {
    /// The unique identifier of the node.
    pub node_id: String,
    /// The address (ip:port) where the node can be reached.
    pub address: SocketAddr,
    /// The last time the node updated its heartbeat.
    pub last_updated: OffsetDateTime,
}

impl NodeInfo {
    fn str(&self, role: &str) -> String {
        let ts = self
            .last_updated
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|e| panic!("Failed to format timestamp: {e}"));

        format!("{role}={},{ts},{}", self.address, self.node_id)
    }
}

/// Result of querying cluster node information.
#[derive(Debug, Clone)]
pub struct ClusterInfo {
    /// The leader node, if one exists.
    pub leader: Option<NodeInfo>,
    /// All follower nodes.
    pub followers: Vec<NodeInfo>,
}

impl ClusterInfo {
    /// Returns true if a leader with the given node_id exists.
    pub fn has_leader(&self, node_id: &str) -> bool {
        self.leader.as_ref().is_some_and(|l| l.node_id == node_id)
    }

    /// Returns true if a follower with the given node_id exists.
    pub fn has_follower(&self, node_id: &str) -> bool {
        self.followers.iter().any(|f| f.node_id == node_id)
    }

    /// Formats the cluster info as a string suitable for writing to a file.
    /// Each node is written on a separate line in the format `role=address,timestamp,node_id`.
    pub fn to_file_content(&self) -> String {
        let mut lines = Vec::new();

        if let Some(leader) = &self.leader {
            lines.push(leader.str("leader"));
        }

        for follower in &self.followers {
            lines.push(follower.str("follower"));
        }

        lines.join("\n")
    }

    /// Parses cluster info from file content.
    /// Each line should be in the format `role=address,timestamp,node_id`.
    pub fn parse(content: &str) -> Result<Self> {
        let mut leader = None;
        let mut followers = Vec::new();

        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("leader=") {
                leader = Some(NodeInfo::parse(rest)?);
            } else if let Some(rest) = line.strip_prefix("follower=") {
                followers.push(NodeInfo::parse(rest)?);
            }
        }

        Ok(ClusterInfo { leader, followers })
    }
}

impl NodeInfo {
    /// Parses a node info from a string in the format `address,timestamp,node_id`.
    pub fn parse(s: &str) -> Result<Self> {
        let mut parts = s.split(',');
        let addr = parts.next().context("Missing address")?;
        let ts = parts.next().context("Missing timestamp")?;
        let node_id = parts.next().context("Missing node_id")?;

        Ok(NodeInfo {
            node_id: node_id.to_string(),
            address: addr.parse().context("Invalid address")?,
            last_updated: OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339)
                .context("Invalid timestamp")?,
        })
    }
}

/// Client for querying cluster information from the database.
pub struct NodeDiscovery {
    max_age: Duration,
    pool: PgPool,
    connection_string: String,
    file_saved_sender: watch::Sender<()>,
}

impl NodeDiscovery {
    /// Creates a new NodeDiscovery with a connection pool and default max_age.
    ///
    /// Returns the NodeDiscovery instance and a receiver that gets notified
    /// whenever the cluster info file is successfully saved.
    pub async fn new(connection_string: &str) -> Result<(Self, watch::Receiver<()>)> {
        Self::new_with_max_age(connection_string, MAX_AGE).await
    }

    /// Creates a new NodeDiscovery with a connection pool and custom max_age.
    ///
    /// The `max_age` parameter controls how long a node can go without updating
    /// its heartbeat before being filtered out of the cluster info.
    ///
    /// Returns the NodeDiscovery instance and a receiver that gets notified
    /// whenever the cluster info file is successfully saved.
    pub async fn new_with_max_age(
        connection_string: &str,
        max_age: Duration,
    ) -> Result<(Self, watch::Receiver<()>)> {
        tracing::info!("Connecting to database.");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(connection_string)
            .await?;
        tracing::info!("DB connection established.");
        let (file_saved_sender, file_saved_receiver) = watch::channel(());

        Ok((
            Self {
                max_age,
                pool,
                connection_string: connection_string.to_string(),
                file_saved_sender,
            },
            file_saved_receiver,
        ))
    }

    // Fetches cluster info atomically using a single transaction.
    // Only nodes whose `last_updated` timestamp is within `max_age` are returned.
    //
    // # Returns
    // A `ClusterInfo` struct containing the leader and all followers.
    async fn get_cluster_info(&self) -> anyhow::Result<ClusterInfo> {
        let (leader_id, all_nodes) = self.get_cluster_info_from_db().await?;
        Self::cluster(leader_id, all_nodes)
    }

    fn cluster(
        leader_id: Option<String>,
        all_nodes: Vec<(String, String, OffsetDateTime)>,
    ) -> anyhow::Result<ClusterInfo> {
        let cap = all_nodes.capacity();
        let mut leader = None;
        let mut followers = Vec::with_capacity(cap);
        let mut seen_node_ids: HashSet<String> = HashSet::with_capacity(cap);
        for (node_id, address, last_updated) in all_nodes {
            if !seen_node_ids.insert(node_id.clone()) {
                anyhow::bail!("Duplicate node id found in Nodes table: {node_id}");
            }

            let address: SocketAddr = address.parse()?;
            let node = NodeInfo {
                node_id,
                address,
                last_updated,
            };

            if Some(&node.node_id) == leader_id.as_ref() {
                leader = Some(node);
            } else {
                followers.push(node);
            }
        }

        if followers.is_empty() {
            if let Some(leader) = &leader {
                followers.push(leader.clone());
            }
        }

        if let Some(leader_id) = &leader_id {
            if leader.is_none() {
                anyhow::bail!("Leader is missing from the Nodes table. leader_id: {leader_id}");
            }
        }

        Ok(ClusterInfo { leader, followers })
    }

    /// Subscribes to PostgreSQL notifications for cluster changes and writes
    /// cluster info to a file whenever the cluster state changes.
    ///
    /// Listens on `nodes_changes` and `leader_changes` channels.
    pub async fn subscribe_cluster_info_loop(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<()> {
        let path = path.as_ref();

        let mut listener = PgListener::connect(&self.connection_string).await?;

        listener
            .listen_all(["nodes_changes", "leader_changes"])
            .await?;

        tracing::info!("Subscribed to nodes_changes and leader_changes channels");

        // Fetch initial cluster info.
        match self.get_cluster_info().await {
            Ok(info) => {
                if let Err(error) = write_to_file(path, info.to_file_content()).await {
                    tracing::warn!(?error, ?path, "Failed to update the cluster info file.");
                } else {
                    // Notify watchers that the file was saved successfully.
                    let _ = self.file_saved_sender.send(());
                }
            }
            Err(error) => {
                tracing::warn!(?error, "Failed to fetch initial cluster info");
            }
        }

        let mut consecutive_errors: u32 = 0;

        loop {
            match self.handle_cluster_update(&mut listener, path).await {
                Ok(()) => consecutive_errors = 0,
                Err(error) => {
                    tracing::warn!(?error, "Cluster update failed");
                    consecutive_errors += 1;
                    // In case of an error, wait a little before querying the database again.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }

            if consecutive_errors >= MAX_DB_ERRORS_ALLOWED {
                tracing::error!(
                    consecutive_errors,
                    "Too many consecutive errors, exiting the node discovery loop."
                );
                anyhow::bail!("Node discovery failed.");
            }
        }
    }

    async fn handle_cluster_update(
        &self,
        listener: &mut PgListener,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        // Wait for at least one notification.
        listener.recv().await?;

        // Drain any additional pending notifications.
        while listener.next_buffered().is_some() {}

        let info = self.get_cluster_info().await?;
        write_to_file(path, info.to_file_content()).await?;
        // Notify watchers that the file was saved successfully.
        let _ = self.file_saved_sender.send(());
        Ok(())
    }

    async fn get_cluster_info_from_db(
        &self,
    ) -> Result<(Option<String>, Vec<(String, String, OffsetDateTime)>)> {
        let max_age_secs = self.max_age.as_secs() as i64;

        // Fetch nodes updated within max_age, always including the leader regardless of age.
        // The leader_id is included in each row via LEFT JOIN, allowing us to get it from the results.
        let rows: Vec<(String, String, OffsetDateTime, Option<String>)> = sqlx::query_as(
            "SELECT n.node_id, n.address, n.last_updated, l.node_id as leader_id \
             FROM nodes n \
             LEFT JOIN sequencer_leader l ON l.singleton = 1 \
             WHERE n.last_updated > NOW() - $1 * INTERVAL '1 second' \
                OR n.node_id = l.node_id \
             ORDER BY n.node_id",
        )
        .bind(max_age_secs)
        .fetch_all(&self.pool)
        .await?;

        // Extract leader_id from first row (same in all rows due to LEFT JOIN)
        let leader_id = rows.first().and_then(|(_, _, _, lid)| lid.clone());

        // Convert to the expected format without leader_id column
        let all_nodes = rows
            .into_iter()
            .map(|(node_id, address, last_updated, _)| (node_id, address, last_updated))
            .collect();

        Ok((leader_id, all_nodes))
    }
}

async fn write_to_file(path: &Path, content: String) -> anyhow::Result<()> {
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("Failed to write cluster info to file at {path:?}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_timestamp() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }

    const TEST_TIME_STAMP: &str = "1970-01-01T00:00:00Z";

    #[test]
    fn cluster_with_leader_and_followers() {
        let ts = test_timestamp();
        let info = NodeDiscovery::cluster(
            Some("node1".to_string()),
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
                ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
                ("node3".to_string(), "127.0.0.1:8002".to_string(), ts),
            ],
        )
        .unwrap();

        assert_eq!(
            info.to_file_content(),
            format!(
                "leader=127.0.0.1:8000,{TEST_TIME_STAMP},node1\n\
                 follower=127.0.0.1:8001,{TEST_TIME_STAMP},node2\n\
                 follower=127.0.0.1:8002,{TEST_TIME_STAMP},node3"
            ),
        );
    }

    #[test]
    fn cluster_with_only_leader_adds_leader_to_followers() {
        let ts = test_timestamp();
        let info = NodeDiscovery::cluster(
            Some("node1".to_string()),
            vec![("node1".to_string(), "127.0.0.1:8000".to_string(), ts)],
        )
        .unwrap();

        assert_eq!(
            info.to_file_content(),
            format!(
                "leader=127.0.0.1:8000,{TEST_TIME_STAMP},node1\n\
                 follower=127.0.0.1:8000,{TEST_TIME_STAMP},node1"
            ),
        );
    }

    #[test]
    fn cluster_with_no_leader() {
        let ts = test_timestamp();
        let info = NodeDiscovery::cluster(
            None,
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
                ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
            ],
        )
        .unwrap();

        assert_eq!(
            info.to_file_content(),
            format!(
                "follower=127.0.0.1:8000,{TEST_TIME_STAMP},node1\n\
                 follower=127.0.0.1:8001,{TEST_TIME_STAMP},node2"
            ),
        );
    }

    #[test]
    fn cluster_empty() {
        let info = NodeDiscovery::cluster(None, vec![]).unwrap();
        assert_eq!(info.to_file_content(), "");
    }

    #[test]
    fn cluster_errors_on_duplicate_node_id() {
        let ts = test_timestamp();
        let result = NodeDiscovery::cluster(
            None,
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
                ("node1".to_string(), "127.0.0.1:8001".to_string(), ts),
            ],
        );

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Duplicate node id found in Nodes table: node1"));
    }

    #[test]
    fn cluster_errors_when_leader_not_in_nodes() {
        let ts = test_timestamp();
        let result = NodeDiscovery::cluster(
            Some("missing_leader".to_string()),
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
                ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
            ],
        );

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Leader is missing from the Nodes table."));
    }
}
