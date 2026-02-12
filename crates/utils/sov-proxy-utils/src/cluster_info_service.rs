use crate::node_discovery::ClusterInfo;
use crate::node_discovery::ClusterUpdateNotifier;
use crate::node_discovery::NodeDiscovery;
use crate::node_discovery::NodeDiscoveryTask;
use crate::root_hash_checker::ClusterRootHashChecker;
use crate::root_hash_checker::ClusterRootHashCheckerTask;
use anyhow::{Context, Result};
use sov_metrics::init_metrics_tracker;
use sov_metrics::MonitoringConfig;
use std::path::PathBuf;
use std::time::Duration;

/// Service that keeps cluster info updated by running a [`NodeDiscovery`] task.
pub struct ClusterInfoService {
    pub node_discovery_task: NodeDiscoveryTask,
    pub root_hash_checker_task: ClusterRootHashCheckerTask,
    metrics_shutdown_sender: tokio::sync::watch::Sender<()>,
}

impl ClusterInfoService {
    /// Connects to PostgreSQL and starts a background task that tracks cluster updates.
    pub async fn spawn(
        connection_string: &str,
        max_age: Duration,
        path: PathBuf,
        notifier: Option<Box<dyn ClusterUpdateNotifier>>,
    ) -> Result<Self> {
        let (metrics_shutdown_sender, mut metrics_shutdown_receiver) =
            tokio::sync::watch::channel(());
        metrics_shutdown_receiver.mark_unchanged();
        let monitoring_config = MonitoringConfig::standard();
        init_metrics_tracker(&monitoring_config, metrics_shutdown_receiver.clone());

        let node_discovery =
            NodeDiscovery::connect(connection_string, max_age, path, notifier).await?;
        let root_hash_checker = ClusterRootHashChecker::new(node_discovery.receiver.clone())?;

        let node_discovery_task = node_discovery.spawn();
        let root_hash_checker_task = root_hash_checker.spawn();

        Ok(Self {
            node_discovery_task,
            root_hash_checker_task,
            metrics_shutdown_sender,
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
        self.metrics_shutdown_sender.send(()).unwrap();
        self.root_hash_checker_task.abort();
        self.node_discovery_task.abort();
    }

    // Waits for the background cluster-info tasks to finish.
    pub async fn join(self) -> anyhow::Result<()> {
        self.root_hash_checker_task.handle.await?;
        self.node_discovery_task.handle.await??;
        Ok(())
    }
}
