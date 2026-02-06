//! Proxy utilities for querying node information from the database.
//!
//! This crate provides the [`Proxy`] struct to retrieve leader and follower
//! IP addresses from the PostgreSQL database atomically.

mod file_writer;

use anyhow::{Context, Result};
use async_trait::async_trait;
use file_writer::write_to_file_atomically;
use sqlx::postgres::{PgListener, PgPool};
use sqlx::FromRow;
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::time::Duration;
pub use time::OffsetDateTime;
use tokio::sync::watch;

const MAX_DB_ERRORS_ALLOWED: u32 = 10;

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
    /// Set of all node IDs in the cluster, used to detect membership changes.
    members: BTreeSet<String>,
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
        let mut members = BTreeSet::new();

        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("leader=") {
                let node = NodeInfo::parse(rest)?;
                members.insert(node.node_id.clone());
                leader = Some(node);
            } else if let Some(rest) = line.strip_prefix("follower=") {
                let node = NodeInfo::parse(rest)?;
                members.insert(node.node_id.clone());
                followers.push(node);
            }
        }

        Ok(ClusterInfo {
            leader,
            followers,
            members,
        })
    }

    fn leader_id(&self) -> Option<String> {
        self.leader.as_ref().map(|l| l.node_id.clone())
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

/// Trait for receiving notifications when cluster state changes.
///
/// Implement this trait to define custom behavior when the cluster membership
/// or leader changes. The notification is triggered after the cluster info file
/// has been updated.
#[async_trait]
pub trait ClusterUpdateNotifier: Send + Sync + 'static {
    /// Called when cluster membership or leadership changes.
    async fn on_cluster_update(&self, cluster_info: &ClusterInfo);
}

/// A simple notifier that signals a watch channel when the cluster updates.
///
/// This is the default implementation of [`ClusterUpdateNotifier`] that uses
/// a [`tokio::sync::watch`] channel to notify waiters of cluster changes.
pub struct SimpleClusterUpdateNotifier {
    sender: watch::Sender<()>,
}

impl SimpleClusterUpdateNotifier {
    /// Creates a new notifier and its corresponding receiver.
    pub fn new() -> (Self, watch::Receiver<()>) {
        let (sender, receiver) = watch::channel(());
        (Self { sender }, receiver)
    }
}

#[async_trait]
impl ClusterUpdateNotifier for SimpleClusterUpdateNotifier {
    async fn on_cluster_update(&self, _cluster_info: &ClusterInfo) {
        let _ = self.sender.send(());
    }
}

/// Client for querying cluster information from the database.
pub struct NodeDiscovery {
    max_age: Duration,
    pool: PgPool,
    connection_string: String,
    prev_members: BTreeSet<String>,
    prev_leader_id: Option<String>,
    notifier: Box<dyn ClusterUpdateNotifier>,
}

impl NodeDiscovery {
    /// Creates a new NodeDiscovery with a connection pool and custom max_age.
    ///
    /// The `max_age` parameter controls how long a node can go without updating
    /// its heartbeat before being filtered out of the cluster info.
    ///
    /// Returns the NodeDiscovery instance and a receiver that gets notified
    /// whenever the cluster info file is successfully saved.
    pub async fn new(
        connection_string: &str,
        max_age: Duration,
        notifier: Box<dyn ClusterUpdateNotifier>,
    ) -> Result<Self> {
        tracing::info!("Connecting to database.");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(connection_string)
            .await?;
        tracing::info!("DB connection established.");

        Ok(Self {
            max_age,
            pool,
            connection_string: connection_string.to_string(),
            prev_members: BTreeSet::new(),
            prev_leader_id: None,
            notifier,
        })
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
        let mut members = BTreeSet::new();
        for (node_id, address, last_updated) in all_nodes {
            if !members.insert(node_id.clone()) {
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

        Ok(ClusterInfo {
            leader,
            followers,
            members,
        })
    }

    /// Subscribes to PostgreSQL notifications for cluster changes and writes
    /// cluster info to a file whenever the cluster state changes.
    ///
    /// Listens on `nodes_changes` and `leader_changes` channels.
    pub async fn subscribe_cluster_info_loop(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<()> {
        let path = path.as_ref();

        let mut listener = PgListener::connect(&self.connection_string).await?;

        listener
            .listen_all(["nodes_changes", "leader_changes"])
            .await?;

        tracing::info!("Subscribed to nodes_changes and leader_changes channels");

        let mut consecutive_errors: u32 = 0;

        // On startup, write an empty file. If the cluster is not empty, the file will be populated on the first call to `handle_cluster_update`.
        write_to_file_atomically(path, "").await?;

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
        &mut self,
        listener: &mut PgListener,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let info = self.get_cluster_info().await?;
        let leader_id = info.leader_id();

        let membership_changed = self.prev_members != info.members;
        let leader_changed = self.prev_leader_id != leader_id;

        if membership_changed || leader_changed {
            // Log membership changes
            for node_id in info.members.difference(&self.prev_members) {
                tracing::info!(node_id, "Node joined the cluster");
            }
            for node_id in self.prev_members.difference(&info.members) {
                tracing::info!(node_id, "Node left the cluster");
            }

            // Log leader change
            if leader_changed {
                tracing::info!(
                    old_leader = ?self.prev_leader_id,
                    new_leader = ?leader_id,
                    "Leader changed"
                );
            }

            let content = info.to_file_content();
            write_to_file_atomically(path, &content).await?;
            tracing::info!(?path, content, "Cluster info file updated");

            self.prev_members = info.members.clone();
            self.prev_leader_id = leader_id;

            // Notify watchers that the cluster was updated.
            self.notifier.on_cluster_update(&info).await;
        }

        tracing::trace!(info = ?info, "Last cluster info");

        // Wait for at least one notification.
        listener.recv().await?;

        // Drain any additional pending notifications.
        while listener.next_buffered().is_some() {}

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

#[cfg(test)]
mod tests;
