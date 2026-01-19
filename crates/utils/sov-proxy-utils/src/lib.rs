//! Proxy utilities for querying node information from the database.
//!
//! This crate provides the [`Proxy`] struct to retrieve leader and follower
//! IP addresses from the PostgreSQL database atomically.

use std::net::SocketAddr;

use anyhow::Result;
use sqlx::postgres::PgPool;
use sqlx::FromRow;

/// Information about a registered node.
#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
pub struct NodeInfo {
    /// The unique identifier of the node.
    pub node_id: String,
    /// The address (ip:port) where the node can be reached.
    pub address: SocketAddr,
}

/// Result of querying cluster node information.
#[derive(Debug, Clone)]
pub struct ClusterInfo {
    /// The leader node, if one exists.
    pub leader: Option<NodeInfo>,
    /// All follower nodes.
    pub followers: Vec<NodeInfo>,
}

/// Client for querying cluster information from the database.
pub struct NodeDiscovery {
    pool: PgPool,
}

impl NodeDiscovery {
    /// Creates a new NodeDiscovery with a connection pool.
    pub async fn new(connection_string: &str) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(connection_string)
            .await?;
        Ok(Self { pool })
    }

    /// Fetches cluster info atomically using a single transaction.
    ///
    /// # Returns
    /// A `ClusterInfo` struct containing the leader and all followers.
    pub async fn get_cluster_info(&self) -> Result<ClusterInfo> {
        let (leader_id, all_nodes) = self.get_cluster_info_from_db().await?;

        let mut leader = None;
        let mut followers = Vec::new();
        for (node_id, address) in all_nodes {
            let address: SocketAddr = address.parse()?;

            let node = NodeInfo { node_id, address };

            if Some(&node.node_id) == leader_id.as_ref() {
                leader = Some(node);
            } else {
                followers.push(node);
            }
        }

        Ok(ClusterInfo { leader, followers })
    }

    /// Periodically writes cluster info to a file in a loop.
    ///
    /// Fetches `ClusterInfo` at the specified interval and writes it
    /// to the given file path using debug format. This method runs
    /// indefinitely - the caller is responsible for cancellation.
    ///
    /// # Arguments
    /// * `path` - File path to write cluster info to
    /// * `interval` - Duration between writes
    pub async fn write_cluster_info_loop(
        &self,
        path: impl AsRef<std::path::Path>,
        interval: std::time::Duration,
    ) {
        let mut interval_timer = tokio::time::interval(interval);
        let path = path.as_ref();

        loop {
            interval_timer.tick().await;

            match self.get_cluster_info().await {
                Ok(info) => {
                    let content = format!("{info:?}");
                    if let Err(e) = std::fs::write(path, content) {
                        tracing::warn!("Failed to write cluster info to file: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch cluster info: {e}");
                }
            }
        }
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
