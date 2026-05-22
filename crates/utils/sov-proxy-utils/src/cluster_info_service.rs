use crate::node_checker::NodeChecker;
use crate::node_checker::NodeCheckerTask;
use crate::node_discovery::ClusterInfo;
use crate::node_discovery::ClusterUpdateNotifier;
use crate::node_discovery::NodeDiscovery;
use crate::node_discovery::NodeDiscoveryTask;
use crate::node_discovery::DEFAULT_POLL_INTERVAL;
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
        install_panic_handler();

        let node_discovery =
            NodeDiscovery::connect(connection_string, max_age, DEFAULT_POLL_INTERVAL, notifier)
                .await?;
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
        self.node_discovery_task.handle.await?;
        Ok(())
    }
}

/// Installs a process-wide panic hook that terminates the process on any panic.
///
/// `tokio::spawn` catches panics at the task boundary: a panicking discovery
/// task would silently stop while the rest of the process keeps running, with
/// the failure surfacing only as a `JoinError` nobody awaits. This hook makes
/// such a panic fatal — it still runs the default hook (so the panic message
/// and backtrace are printed) and then exits with a non-zero status.
///
/// Installed only once, even if [`ClusterInfoService::spawn`] is called
/// repeatedly.
fn install_panic_handler() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            default_hook(info);
            std::process::exit(1);
        }));
    });
}
