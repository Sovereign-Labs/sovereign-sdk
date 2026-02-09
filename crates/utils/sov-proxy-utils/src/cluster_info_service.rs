use crate::node_discovery::ClusterInfo;
use crate::node_discovery::ClusterUpdateNotifier;
use crate::node_discovery::NodeDiscovery;
use crate::node_discovery::NodeDiscoveryTask;
use crate::root_hash_checker::ClusterRootHashChecker;
use crate::root_hash_checker::ClusterRootHashCheckerTask;
use crate::root_hash_checker::RootHashCheck;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::watch;

/// Service that keeps cluster info updated by running a [`NodeDiscovery`] task.
pub struct ClusterInfoService {
    node_discovery_task: NodeDiscoveryTask,
    root_hash_checker_task: ClusterRootHashCheckerTask,
}

impl ClusterInfoService {
    /// Connects to PostgreSQL and starts a background task that tracks cluster updates.
    pub async fn spawn(
        connection_string: &str,
        max_age: Duration,
        path: PathBuf,
        notifier: Option<Box<dyn ClusterUpdateNotifier>>,
    ) -> Result<Self> {
        let node_discovery =
            NodeDiscovery::connect(connection_string, max_age, path, notifier).await?;
        let node_discovery_task = node_discovery.spawn();

        let root_hash_checker_task =
            ClusterRootHashChecker::new(node_discovery_task.receiver.clone()).spawn();

        Ok(Self {
            node_discovery_task,
            root_hash_checker_task,
        })
    }

    /// Returns a new watcher subscription for cluster info changes.
    pub fn subscribe(&self) -> watch::Receiver<ClusterInfo> {
        self.node_discovery_task.receiver.clone()
    }

    /// Returns a watcher subscription for root-hash check results.
    pub fn subscribe_root_hash_checks(&self) -> watch::Receiver<RootHashCheck> {
        self.root_hash_checker_task.receiver.clone()
    }

    /// Waits for the next cluster info update with timeout.
    pub async fn wait_for_update_with_timeout(&mut self, timeout: Duration) -> Result<ClusterInfo> {
        tokio::time::timeout(timeout, self.node_discovery_task.receiver.changed())
            .await
            .context("Timed out waiting for cluster info update")?
            .context("Cluster info watcher closed")?;

        Ok(self
            .node_discovery_task
            .receiver
            .borrow_and_update()
            .clone())
    }

    /// Stops the background cluster-info task.
    pub fn shutdown(self) {
        self.root_hash_checker_task.handle.abort();
        self.node_discovery_task.handle.abort();
    }
}
