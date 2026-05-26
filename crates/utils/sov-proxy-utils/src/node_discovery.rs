//! Utilities for discovering cluster nodes from PostgreSQL and persisting snapshots.
use crate::node_discovery_metrics::{ClusterUpdateFailureMetric, ClusterUpdateMetric};
use anyhow::Result;
use async_trait::async_trait;
use sqlx::postgres::{PgListener, PgPool};
use sqlx::FromRow;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
pub use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Re-emit the cluster update metric at least every `LIVENESS_POLL_MULTIPLIER
/// * poll_interval` even if the cluster is unchanged, so the absence of
/// samples can be alerted on if the polling task dies silently.
const LIVENESS_POLL_MULTIPLIER: u32 = 5;

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

/// Result of querying cluster node information.
#[derive(Debug, Clone, Default)]
pub struct ClusterInfo {
    /// The leader node, if one exists.
    pub leader: Option<NodeInfo>,
    /// All follower nodes.
    pub followers: BTreeMap<String, NodeInfo>,
}

impl ClusterInfo {
    /// Returns true if a leader with the given node_id exists.
    pub fn has_leader(&self, node_id: &str) -> bool {
        self.leader.as_ref().is_some_and(|l| l.node_id == node_id)
    }

    /// Returns true if a follower with the given node_id exists.
    pub fn has_follower(&self, node_id: &str) -> bool {
        self.followers.contains_key(node_id)
    }

    fn leader_id(&self) -> Option<String> {
        self.leader.as_ref().map(|l| l.node_id.clone())
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
    async fn on_cluster_update(&mut self, cluster_info: &ClusterInfo) -> anyhow::Result<()>;
}

/// Handle returned when subscribing to cluster updates.
pub struct NodeDiscoveryTask {
    pub receiver: watch::Receiver<ClusterInfo>,
    pub(crate) handle: JoinHandle<()>,
}

impl NodeDiscoveryTask {
    pub fn abort(&self) {
        self.handle.abort();
    }
}

#[derive(Debug, thiserror::Error)]
enum HandleClusterUpdateError {
    #[error("Failed to connect to the cluster database")]
    Connect(#[source] anyhow::Error),
    #[error("Failed to fetch cluster info")]
    GetClusterInfo(#[source] anyhow::Error),
    #[error("Failed to notify on cluster update")]
    Notify(#[source] anyhow::Error),
    #[error("Failed to receive cluster notification")]
    RecvNotification(#[source] anyhow::Error),
}

impl HandleClusterUpdateError {
    fn stage(&self) -> &'static str {
        match self {
            Self::Connect(_) => "connect",
            Self::GetClusterInfo(_) => "get_cluster_info",
            Self::Notify(_) => "notify",
            Self::RecvNotification(_) => "recv_notification",
        }
    }

    /// Logs the failure and records a failure metric.
    fn report(&self) {
        let stage = self.stage();
        tracing::warn!(stage, error = ?self, "Cluster update failed");
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(ClusterUpdateFailureMetric { stage });
        });
    }

    /// Logs the failure, records a failure metric, and backs off before the
    /// caller's next retry.
    async fn report_and_backoff(&self) {
        self.report();
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

/// Default upper bound on how long to wait before re-polling cluster info.
pub(crate) const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Client for querying cluster information from the database.
pub struct NodeDiscovery {
    max_age: Duration,
    poll_interval: Duration,
    liveness_interval: Duration,
    pool: PgPool,
    listener: PgListener,
    prev_followers: BTreeSet<String>,
    prev_leader_id: Option<String>,
    notifier: Option<Box<dyn ClusterUpdateNotifier>>,
    sender: watch::Sender<ClusterInfo>,
    pub(crate) receiver: watch::Receiver<ClusterInfo>,
    last_metric_at: Option<Instant>,
}

impl NodeDiscovery {
    /// Creates a new NodeDiscovery with a connection pool and custom max_age.
    ///
    /// The `max_age` parameter controls how long a node can go without updating
    /// its heartbeat before being filtered out of the cluster info.
    ///
    /// Returns a [`NodeDiscovery`] ready to subscribe for cluster updates.
    pub async fn connect(
        connection_string: &str,
        max_age: Duration,
        poll_interval: Duration,
        notifier: Option<Box<dyn ClusterUpdateNotifier>>,
    ) -> Result<Self> {
        Self::try_connect(connection_string, max_age, poll_interval, notifier)
            .await
            .map_err(|error| {
                let error = HandleClusterUpdateError::Connect(error);
                error.report();
                anyhow::Error::new(error)
            })
    }

    async fn try_connect(
        connection_string: &str,
        max_age: Duration,
        poll_interval: Duration,
        notifier: Option<Box<dyn ClusterUpdateNotifier>>,
    ) -> Result<Self> {
        tracing::info!("Connecting to database.");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(connection_string)
            .await?;
        tracing::info!("DB connection established.");

        let mut listener = PgListener::connect(connection_string).await?;
        listener
            .listen_all(["nodes_changes", "leader_changes"])
            .await?;
        tracing::info!("Subscribed to nodes_changes and leader_changes channels");

        let (sender, receiver) = watch::channel(ClusterInfo::default());

        Ok(Self {
            max_age,
            poll_interval,
            liveness_interval: poll_interval * LIVENESS_POLL_MULTIPLIER,
            pool,
            listener,
            prev_followers: BTreeSet::new(),
            prev_leader_id: None,
            notifier,
            sender,
            receiver,
            last_metric_at: None,
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
        let mut leader = None;
        let mut followers = BTreeMap::new();

        for (node_id, address, last_updated) in all_nodes {
            let address: SocketAddr = address.parse()?;
            let node = NodeInfo {
                node_id,
                address,
                last_updated,
            };

            if Some(&node.node_id) == leader_id.as_ref() {
                leader = Some(node);
            } else {
                match followers.entry(node.node_id.clone()) {
                    Entry::Vacant(vacant_entry) => {
                        vacant_entry.insert(node);
                    }
                    Entry::Occupied(occupied) => {
                        anyhow::bail!("Duplicate node id found in Nodes table: {}", occupied.key());
                    }
                }
            }
        }

        if let Some(leader_id) = &leader_id {
            if leader.is_none() {
                anyhow::bail!("Leader is missing from the Nodes table. leader_id: {leader_id}, followers: {followers:?}");
            }
        }

        Ok(ClusterInfo { leader, followers })
    }

    /// Subscribes to PostgreSQL notifications for cluster changes and writes
    /// cluster info to a file whenever the cluster state changes.
    ///
    /// Listens on `nodes_changes` and `leader_changes` channels.
    pub fn spawn(mut self) -> NodeDiscoveryTask {
        let receiver = self.receiver.clone();
        let handle = tokio::spawn(async move {
            loop {
                if let Err(error) = self.handle_cluster_update().await {
                    error.report_and_backoff().await;
                }

                // Wait for the next change signal, or re-poll after poll_interval.
                if let Err(error) = self.recv_notification().await {
                    error.report_and_backoff().await;
                }

                // Drain any additional pending notifications.
                while self.listener.next_buffered().is_some() {}
            }
        });

        NodeDiscoveryTask { receiver, handle }
    }

    /// Waits once for the next cluster change signal before returning to the
    /// caller's loop, which always re-polls cluster info.
    async fn recv_notification(&mut self) -> Result<(), HandleClusterUpdateError> {
        let poll_interval = self.poll_interval;
        match tokio::time::timeout(poll_interval, self.listener.try_recv()).await {
            // A notification arrived — re-poll.
            Ok(Ok(Some(_))) => {}
            // The listener connection dropped and was re-established. Any
            // notifications sent before LISTEN was restored may have been lost,
            // so re-poll cluster info immediately instead of waiting for the
            // next poll interval.
            Ok(Ok(None)) => {
                tracing::debug!(
                    "Cluster notification listener reconnected after connection loss, re-polling"
                );
            }
            // No notification arrived within `poll_interval`. This is expected
            // when the cluster is quiet; we re-poll anyway to bound staleness.
            Err(_) => {
                tracing::debug!(
                    ?poll_interval,
                    "No cluster notification received within the poll interval, re-polling"
                );
            }
            Ok(Err(error)) => {
                return Err(HandleClusterUpdateError::RecvNotification(error.into()));
            }
        }
        Ok(())
    }

    async fn handle_cluster_update(&mut self) -> Result<(), HandleClusterUpdateError> {
        let info = self
            .get_cluster_info()
            .await
            .map_err(HandleClusterUpdateError::GetClusterInfo)?;
        let leader_id = info.leader_id();

        let followers: BTreeSet<_> = info.followers.keys().cloned().collect();

        let membership_changed = self.prev_followers != followers;
        let leader_changed = self.prev_leader_id != leader_id;
        let cluster_changed = membership_changed || leader_changed;

        // Liveness is keyed off wall-clock time, not the wake-up source, so a
        // chatty cluster (frequent NOTIFY traffic) still emits periodic samples
        // and missing-sample alerts only fire if the polling task actually
        // stops making progress.
        let liveness_due = self
            .last_metric_at
            .is_none_or(|t| t.elapsed() >= self.liveness_interval);

        if !(cluster_changed || liveness_due) {
            return Ok(());
        }

        tracing::debug!(info = ?info, membership_changed, leader_changed, "Last cluster info");

        let update_metric = ClusterUpdateMetric {
            current_leader: leader_id.clone(),
            followers: followers.iter().cloned().collect(),
            cluster_changed,
        };

        if cluster_changed {
            // Log membership changes
            for node_id in followers.difference(&self.prev_followers) {
                tracing::info!(node_id, "Node joined the followers");
            }
            for node_id in self.prev_followers.difference(&followers) {
                tracing::info!(node_id, "Node left the followers");
            }

            // Log leader change
            if leader_changed {
                tracing::info!(
                    old_leader = ?self.prev_leader_id,
                    new_leader = ?leader_id,
                    "Leader changed"
                );
            }

            // Notify watchers that the cluster was updated.
            if let Some(notifier) = &mut self.notifier {
                notifier
                    .on_cluster_update(&info)
                    .await
                    .map_err(HandleClusterUpdateError::Notify)?;
            }

            self.prev_followers = followers;
            self.prev_leader_id = leader_id;
            let _ = self.sender.send(info);
        }

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(update_metric);
        });
        self.last_metric_at = Some(Instant::now());

        Ok(())
    }

    async fn get_cluster_info_from_db(
        &self,
    ) -> Result<(Option<String>, Vec<(String, String, OffsetDateTime)>)> {
        let max_age_secs: i64 = self.max_age.as_secs().try_into()?;

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
mod tests {
    use super::*;

    fn test_timestamp() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }

    #[test]
    fn cluster_errors_on_duplicate_node_id() {
        let ts = test_timestamp();
        let result = NodeDiscovery::cluster(
            None,
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
                ("node1".to_string(), "127.0.0.1:8001".to_string(), ts),
            ],
        );

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Duplicate node id found in Nodes table: node1"));
    }

    #[test]
    fn cluster_errors_when_leader_not_in_nodes() {
        let ts = test_timestamp();
        let result = NodeDiscovery::cluster(
            Some("missing_leader".to_string()),
            vec![
                ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
                ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
            ],
        );

        let err = result.unwrap_err();
        assert!(err
            .to_string()
            .contains("Leader is missing from the Nodes table."));
    }

    #[test]
    fn leader_change_detected_when_membership_unchanged() {
        let ts = test_timestamp();
        let nodes = vec![
            ("node1".to_string(), "127.0.0.1:8000".to_string(), ts),
            ("node2".to_string(), "127.0.0.1:8001".to_string(), ts),
        ];

        // Initial state: node1 is leader
        let info1 = NodeDiscovery::cluster(Some("node1".to_string()), nodes.clone()).unwrap();

        // New state: node2 becomes leader, same membership
        let info2 = NodeDiscovery::cluster(Some("node2".to_string()), nodes).unwrap();

        // Followers differ because the previous leader becomes a follower and vice versa.
        assert_ne!(info1.followers, info2.followers);

        // But leader_id should differ.
        assert_eq!(info1.leader_id(), Some("node1".to_string()));
        assert_eq!(info2.leader_id(), Some("node2".to_string()));
    }
}
