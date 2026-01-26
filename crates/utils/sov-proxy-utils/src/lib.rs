//! Proxy utilities for querying node information from the database.
//!
//! This crate provides the [`Proxy`] struct to retrieve leader and follower
//! IP addresses from the PostgreSQL database atomically.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::{PgListener, PgPool};
use sqlx::FromRow;
use tokio::sync::watch;

const MAX_DB_ERRORS_ALLOWED: u32 = 10;

/// Information about a registered node.
#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct NodeInfo {
    /// The unique identifier of the node.
    pub node_id: String,
    /// The address (ip:port) where the node can be reached.
    pub address: SocketAddr,
}

impl NodeInfo {
    fn str(&self, role: &str) -> String {
        format!("{role}={}", self.address)
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
    /// Formats the cluster info as a string suitable for writing to a file.
    /// Each node is written on a separate line in the format `role=address`.
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
}

/// Client for querying cluster information from the database.
pub struct NodeDiscovery {
    pool: PgPool,
    connection_string: String,
    file_saved_sender: watch::Sender<()>,
}

impl NodeDiscovery {
    /// Creates a new NodeDiscovery with a connection pool.
    ///
    /// Returns the NodeDiscovery instance and a receiver that gets notified
    /// whenever the cluster info file is successfully saved.
    pub async fn new(connection_string: &str) -> Result<(Self, watch::Receiver<()>)> {
        tracing::info!("Connecting to database.");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(connection_string)
            .await?;
        tracing::info!("DB connection established.");
        let (file_saved_sender, file_saved_receiver) = watch::channel(());

        Ok((
            Self {
                pool,
                connection_string: connection_string.to_string(),
                file_saved_sender,
            },
            file_saved_receiver,
        ))
    }

    /// Fetches cluster info atomically using a single transaction.
    ///
    /// # Returns
    /// A `ClusterInfo` struct containing the leader and all followers.
    pub async fn get_cluster_info(&self) -> anyhow::Result<ClusterInfo> {
        let (leader_id, all_nodes) = self.get_cluster_info_from_db().await?;
        Self::cluster(leader_id, all_nodes)
    }

    fn cluster(
        leader_id: Option<String>,
        all_nodes: Vec<(String, String)>,
    ) -> anyhow::Result<ClusterInfo> {
        let cap = all_nodes.capacity();
        let mut leader = None;
        let mut followers = Vec::with_capacity(cap);
        let mut seen_node_ids = HashSet::with_capacity(cap);
        for (node_id, address) in all_nodes {
            if !seen_node_ids.insert(node_id.clone()) {
                anyhow::bail!("Duplicate node id found in Nodes table: {node_id}");
            }

            let address: SocketAddr = address.parse()?;
            let node = NodeInfo { node_id, address };

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

    async fn get_cluster_info_from_db(&self) -> Result<(Option<String>, Vec<(String, String)>)> {
        let mut tx = self.pool.begin().await?;

        // Fetch leader within transaction
        let leader_id: Option<String> =
            sqlx::query_scalar("SELECT node_id FROM sequencer_leader WHERE singleton = 1")
                .fetch_optional(&mut *tx)
                .await?;

        // Fetch all nodes within same transaction
        let all_nodes: Vec<(String, String)> =
            sqlx::query_as("SELECT node_id, address FROM nodes ORDER BY node_id")
                .fetch_all(&mut *tx)
                .await?;

        tx.commit().await?;

        Ok((leader_id, all_nodes))
    }
}

async fn write_to_file(path: impl AsRef<std::path::Path>, content: String) -> anyhow::Result<()> {
    tokio::fs::write(path, content)
        .await
        .context("Failed to write cluster info to file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_with_leader_and_followers() {
        let info = NodeDiscovery::cluster(
            Some("node1".to_string()),
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string()),
                ("node2".to_string(), "127.0.0.1:8001".to_string()),
                ("node3".to_string(), "127.0.0.1:8002".to_string()),
            ],
        )
        .unwrap();

        assert_eq!(
            &info.to_file_content(),
            "leader=127.0.0.1:8000\nfollower=127.0.0.1:8001\nfollower=127.0.0.1:8002",
        );
    }

    #[test]
    fn cluster_with_only_leader_adds_leader_to_followers() {
        let info = NodeDiscovery::cluster(
            Some("node1".to_string()),
            vec![("node1".to_string(), "127.0.0.1:8000".to_string())],
        )
        .unwrap();

        assert_eq!(
            &info.to_file_content(),
            "leader=127.0.0.1:8000\nfollower=127.0.0.1:8000",
        );
    }

    #[test]
    fn cluster_with_no_leader() {
        let info = NodeDiscovery::cluster(
            None,
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string()),
                ("node2".to_string(), "127.0.0.1:8001".to_string()),
            ],
        )
        .unwrap();

        assert_eq!(
            &info.to_file_content(),
            "follower=127.0.0.1:8000\nfollower=127.0.0.1:8001",
        );
    }

    #[test]
    fn cluster_empty() {
        let info = NodeDiscovery::cluster(None, vec![]).unwrap();
        assert_eq!(info.to_file_content(), "");
    }

    #[test]
    fn cluster_errors_on_duplicate_node_id() {
        let result = NodeDiscovery::cluster(
            None,
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string()),
                ("node1".to_string(), "127.0.0.1:8001".to_string()),
            ],
        );

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Duplicate node id found in Nodes table: node1"));
    }

    #[test]
    fn cluster_errors_when_leader_not_in_nodes() {
        let result = NodeDiscovery::cluster(
            Some("missing_leader".to_string()),
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string()),
                ("node2".to_string(), "127.0.0.1:8001".to_string()),
            ],
        );

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Leader is missing from the Nodes table."));
    }
}
