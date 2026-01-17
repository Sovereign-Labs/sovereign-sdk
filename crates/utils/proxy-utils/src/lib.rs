//! Proxy utilities for querying node information from the database.
//!
//! This crate provides functions to retrieve leader and follower
//! IP addresses from the PostgreSQL database.

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

/// Queries the database and returns information about all nodes in the cluster.
///
/// # Arguments
/// * `pool` - A reference to the PostgreSQL connection pool.
///
/// # Returns
/// A `ClusterInfo` struct containing the leader and all followers.
pub async fn get_cluster_info(pool: &PgPool) -> Result<ClusterInfo> {
    // Get the current leader's node_id
    let leader_id: Option<String> =
        sqlx::query_scalar("SELECT node_id FROM sequencer_leader WHERE singleton = 1")
            .fetch_optional(pool)
            .await?;

    // Get all registered nodes
    let all_nodes: Vec<NodeInfo> =
        sqlx::query_as("SELECT node_id, address FROM nodes ORDER BY node_id")
            .fetch_all(pool)
            .await?;

    // Separate leader from followers
    let (leader, followers) = match &leader_id {
        Some(lid) => {
            let leader = all_nodes.iter().find(|n| &n.node_id == lid).cloned();
            let followers = all_nodes
                .into_iter()
                .filter(|n| &n.node_id != lid)
                .collect();
            (leader, followers)
        }
        None => (None, all_nodes),
    };

    Ok(ClusterInfo { leader, followers })
}

/// Queries the database and returns only the leader node address.
///
/// # Arguments
/// * `pool` - A reference to the PostgreSQL connection pool.
///
/// # Returns
/// The leader's address if one exists.
pub async fn get_leader_address(pool: &PgPool) -> Result<Option<String>> {
    let result: Option<String> = sqlx::query_scalar(
        "SELECT n.address
         FROM nodes n
         INNER JOIN sequencer_leader sl ON n.node_id = sl.node_id
         WHERE sl.singleton = 1",
    )
    .fetch_optional(pool)
    .await?;

    Ok(result)
}

/// Queries the database and returns all follower node addresses.
///
/// # Arguments
/// * `pool` - A reference to the PostgreSQL connection pool.
///
/// # Returns
/// A vector of follower addresses.
pub async fn get_follower_addresses(pool: &PgPool) -> Result<Vec<String>> {
    let addresses: Vec<String> = sqlx::query_scalar(
        "SELECT n.address
         FROM nodes n
         WHERE NOT EXISTS (
             SELECT 1 FROM sequencer_leader sl
             WHERE sl.node_id = n.node_id AND sl.singleton = 1
         )
         ORDER BY n.node_id",
    )
    .fetch_all(pool)
    .await?;

    Ok(addresses)
}

/// Creates a connection pool from a connection string.
///
/// # Arguments
/// * `connection_string` - PostgreSQL connection string.
///
/// # Returns
/// A configured PgPool.
pub async fn create_pool(connection_string: &str) -> Result<PgPool> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(connection_string)
        .await?;
    Ok(pool)
}
