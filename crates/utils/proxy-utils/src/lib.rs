//! Proxy utilities for querying node information from the database.
//!
//! This crate provides the [`Proxy`] struct to retrieve leader and follower
//! IP addresses from the PostgreSQL database atomically.

use anyhow::Result;
use sqlx::postgres::PgPool;
use sqlx::FromRow;

/// Information about a registered node.
#[derive(Debug, Clone, FromRow)]
pub struct NodeInfo {
    /// The unique identifier of the node.
    pub node_id: String,
    /// The address (ip:port) where the node can be reached.
    pub address: String,
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
        for node in all_nodes {
            if Some(&node.node_id) == leader_id.as_ref() {
                leader = Some(node);
            } else {
                followers.push(node);
            }
        }

        Ok(ClusterInfo { leader, followers })
    }

    async fn get_cluster_info_from_db(&self) -> Result<(Option<String>, Vec<NodeInfo>)> {
        let mut tx = self.pool.begin().await?;

        // Fetch leader within transaction
        let leader_id: Option<String> =
            sqlx::query_scalar("SELECT node_id FROM sequencer_leader WHERE singleton = 1")
                .fetch_optional(&mut *tx)
                .await?;

        // Fetch all nodes within same transaction
        let all_nodes: Vec<NodeInfo> =
            sqlx::query_as("SELECT node_id, address FROM nodes ORDER BY node_id")
                .fetch_all(&mut *tx)
                .await?;

        tx.commit().await?;
        Ok((leader_id, all_nodes))
    }
}
