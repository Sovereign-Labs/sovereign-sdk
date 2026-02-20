use crate::node_checker::NodeChecker;
use crate::node_checker::NodeCheckerTask;
use crate::node_discovery::ClusterInfo;
use crate::node_discovery::ClusterUpdateNotifier;
use crate::node_discovery::NodeDiscovery;
use crate::node_discovery::NodeDiscoveryTask;
use anyhow::{Context, Result};
use std::time::Duration;

/// Service that keeps cluster info updated by running a [`NodeDiscovery`] task.
pub struct ClusterInfoService {
    pub node_discovery_task: NodeDiscoveryTask,
    pub node_checker_task: NodeCheckerTask,
}

impl ClusterInfoService {
    /// Connects to PostgreSQL and starts a background task that tracks cluster updates.
    pub async fn spawn(
        connection_string: &str,
        max_age: Duration,
        notifier: Option<Box<dyn ClusterUpdateNotifier>>,
    ) -> Result<Self> {
        let node_discovery = NodeDiscovery::connect(connection_string, max_age, notifier).await?;
        let node_checker = NodeChecker::new(node_discovery.receiver.clone())?;

        let node_discovery_task = node_discovery.spawn();
        let node_checker_task = node_checker.spawn();

        Ok(Self {
            node_discovery_task,
            node_checker_task,
        })
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
        self.node_checker_task.abort();
        self.node_discovery_task.abort();
    }

    // Waits for the background cluster-info tasks to finish.
    pub async fn join(self) -> anyhow::Result<()> {
        self.node_checker_task.handle.await?;
        self.node_discovery_task.handle.await??;
        Ok(())
    }
}
