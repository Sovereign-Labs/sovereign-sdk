use std::time::Duration;

use anyhow::Result;
use tokio::sync::watch;

use crate::node_discovery::NodeDiscovery;
use crate::root_hash_checker::{ClusterRootHashChecker, RootHashCheck};
use crate::ClusterInfo;

/// Combines node discovery and root hash checking into a single cluster monitor.
pub struct ClusterMonitor {
    /// Discovers cluster membership and leadership from the database.
    pub node_discovery: NodeDiscovery,
    /// Checks root hash consistency across cluster nodes.
    pub root_hash_checker: ClusterRootHashChecker,
}

impl ClusterMonitor {
    /// Creates a new `ClusterMonitor`.
    ///
    /// Returns the monitor and a receiver that gets notified whenever
    /// the cluster info changes.
    pub async fn new(
        connection_string: &str,
        max_age: Duration,
    ) -> Result<(Self, watch::Receiver<ClusterInfo>)> {
        let (node_discovery, receiver) = NodeDiscovery::new(connection_string, max_age).await?;
        let root_hash_checker = ClusterRootHashChecker::new(receiver.clone())?;

        Ok((
            Self {
                node_discovery,
                root_hash_checker,
            },
            receiver,
        ))
    }

    /// Checks root hash consistency across all nodes using the latest cluster info.
    pub async fn check_root_hashes(&self) -> RootHashCheck {
        self.root_hash_checker.check_root_hashes().await
    }

    /// Subscribes to PostgreSQL notifications for cluster changes and writes
    /// cluster info to a file whenever the cluster state changes.
    pub async fn subscribe_cluster_info_loop(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<()> {
        self.node_discovery.subscribe_cluster_info_loop(path).await
    }
}
