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

/// Re-emit the cluster update metric at least every
/// `poll_interval * LIVENESS_POLL_MULTIPLIER` even if the cluster is unchanged,
/// so the absence of samples can be alerted on if the polling task dies silently.
const LIVENESS_POLL_MULTIPLIER: u32 = 5;
const READY_ENDPOINT_PATH: &str = "/sequencer/ready";
const READY_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const READY_CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ClusterMembership {
    pub(crate) followers: BTreeSet<String>,
    pub(crate) leader_id: Option<String>,
}

impl ClusterMembership {
    fn from_cluster_info(info: &ClusterInfo) -> Self {
        Self {
            followers: info.followers.keys().cloned().collect(),
            leader_id: info.leader_id(),
        }
    }

    fn change_from(&self, previous: &Self) -> ClusterMembershipChange {
        let followers_changed = previous.followers != self.followers;
        let leader_changed = previous.leader_id != self.leader_id;

        ClusterMembershipChange {
            followers_changed,
            leader_changed,
        }
    }
}

struct ClusterMembershipChange {
    followers_changed: bool,
    leader_changed: bool,
}

impl ClusterMembershipChange {
    fn has_changed(&self) -> bool {
        self.followers_changed || self.leader_changed
    }

    fn followers_changed(&self) -> bool {
        self.followers_changed
    }

    fn leader_changed(&self) -> bool {
        self.leader_changed
    }
}

#[derive(Debug, Default)]
struct AdvertisedClusterInfo {
    cluster_info: ClusterInfo,
    /// Followers that responded but reported they are not ready, keyed by node
    /// id with the HTTP status they returned. Disjoint from `errors`: a
    /// follower we could not reach at all is recorded there instead.
    not_ready_followers: BTreeMap<String, reqwest::StatusCode>,
    errors: Vec<FollowerReadinessProbeError>,
}

impl AdvertisedClusterInfo {
    fn log(&self) {
        let errors: Vec<_> = self
            .errors
            .iter()
            .map(FollowerReadinessProbeError::as_log_fields)
            .collect();

        let not_ready_followers: Vec<_> = self
            .not_ready_followers
            .iter()
            .map(|(node_id, status)| (node_id.as_str(), status.as_u16()))
            .collect();

        tracing::debug!(
            advertised_leader = ?self.cluster_info.leader.as_ref().map(|leader| &leader.node_id),
            advertised_followers = ?self.cluster_info.followers.keys().collect::<Vec<_>>(),
            not_ready_followers = ?not_ready_followers,
            errors = ?errors,
            "Advertised cluster info updated"
        );
    }

    fn not_ready_followers_count(&self) -> usize {
        self.not_ready_followers.len()
    }

    fn errors_count(&self) -> usize {
        self.errors.len()
    }
}

/// Whether a follower reported itself ready, and if not, the status it returned.
#[derive(Debug)]
enum FollowerReadiness {
    Ready,
    NotReady(reqwest::StatusCode),
}

#[derive(Debug)]
struct FollowerReadinessProbeError {
    /// The probed node, or `None` when the probe task itself failed to join
    /// (panicked or was cancelled) and no node identity is available.
    node: Option<(String, SocketAddr)>,
    error: String,
}

impl FollowerReadinessProbeError {
    fn as_log_fields(&self) -> (Option<&str>, Option<SocketAddr>, &str) {
        (
            self.node
                .as_ref()
                .map(|(node_id, _address)| node_id.as_str()),
            self.node.as_ref().map(|(_node_id, address)| *address),
            self.error.as_str(),
        )
    }
}

/// Trait for receiving notifications when cluster state changes.
///
/// Implement this trait to define custom behavior when the advertised cluster
/// membership or leader changes.
#[async_trait]
pub trait ClusterUpdateNotifier: Send + Sync + 'static {
    /// Called when advertised cluster membership or leadership changes.
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
    http_client: reqwest::Client,
    prev_membership: ClusterMembership,
    /// Last advertised membership emitted to the notifier, or `None` if nothing
    /// has been emitted yet. `None` forces the first poll to materialize the
    /// current view — even when it is empty — so stale contents left behind by a
    /// previous process are overwritten instead of lingering until the next
    /// real change.
    prev_advertised_membership: Option<ClusterMembership>,
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
        let http_client = reqwest::Client::builder()
            .connect_timeout(READY_CONNECTION_TIMEOUT)
            .timeout(READY_REQUEST_TIMEOUT)
            .build()?;

        Ok(Self {
            max_age,
            poll_interval,
            liveness_interval: poll_interval * LIVENESS_POLL_MULTIPLIER,
            pool,
            listener,
            http_client,
            prev_membership: ClusterMembership::default(),
            prev_advertised_membership: None,
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
        let membership = ClusterMembership::from_cluster_info(&info);
        let membership_change = membership.change_from(&self.prev_membership);

        // Probe readiness on every poll cycle.
        let advertised_info = self.advertised_cluster_info(&info).await;
        let advertised_membership =
            ClusterMembership::from_cluster_info(&advertised_info.cluster_info);
        // Treat the very first poll (`None`) as a change so the advertised view
        // is always materialized once, even if it is empty.
        let advertised_changed = self
            .prev_advertised_membership
            .as_ref()
            .is_none_or(|prev| advertised_membership.change_from(prev).has_changed());

        // Liveness is keyed off wall-clock time, not the wake-up source, so a
        // chatty cluster (frequent NOTIFY traffic) still emits periodic samples
        // and missing-sample alerts only fire if the polling task actually
        // stops making progress.
        let liveness_due = self
            .last_metric_at
            .is_none_or(|t| t.elapsed() >= self.liveness_interval);

        if !(membership_change.has_changed() || advertised_changed || liveness_due) {
            return Ok(());
        }

        tracing::debug!(
            info = ?info,
            followers_changed = membership_change.followers_changed(),
            leader_changed = membership_change.leader_changed(),
            "Last cluster info"
        );

        advertised_info.log();

        let update_metric = ClusterUpdateMetric {
            membership: membership.clone(),
            cluster_changed: membership_change.has_changed(),
            advertised_membership: advertised_membership.clone(),
            advertised_cluster_changed: advertised_changed,
            not_ready_followers_count: advertised_info.not_ready_followers_count(),
            errored_followers_count: advertised_info.errors_count(),
        };

        if advertised_changed {
            if let Some(notifier) = &mut self.notifier {
                notifier
                    .on_cluster_update(&advertised_info.cluster_info)
                    .await
                    .map_err(HandleClusterUpdateError::Notify)?;
            }

            self.prev_advertised_membership = Some(advertised_membership);
        }

        if membership_change.has_changed() {
            self.prev_membership = membership;
            let _ = self.sender.send(info);
        }

        self.last_metric_at = Some(Instant::now());

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(update_metric);
        });

        Ok(())
    }

    async fn advertised_cluster_info(&self, info: &ClusterInfo) -> AdvertisedClusterInfo {
        let mut probes = tokio::task::JoinSet::new();

        for (node_id, node) in &info.followers {
            let http_client = self.http_client.clone();
            let node_id = node_id.clone();
            let node = node.clone();

            probes.spawn(async move {
                let ready = Self::follower_is_ready(&http_client, &node).await;
                (node_id, node, ready)
            });
        }

        let mut followers = BTreeMap::new();
        let mut not_ready_followers = BTreeMap::new();
        let mut errors = Vec::new();

        while let Some(result) = probes.join_next().await {
            match result {
                Ok((node_id, node, Ok(FollowerReadiness::Ready))) => {
                    followers.insert(node_id, node);
                }
                Ok((node_id, _node, Ok(FollowerReadiness::NotReady(status)))) => {
                    not_ready_followers.insert(node_id, status);
                }
                Ok((node_id, node, Err(error))) => {
                    errors.push(FollowerReadinessProbeError {
                        node: Some((node_id, node.address)),
                        error: error.to_string(),
                    });
                }
                Err(error) => {
                    errors.push(FollowerReadinessProbeError {
                        node: None,
                        error: error.to_string(),
                    });
                }
            }
        }

        AdvertisedClusterInfo {
            cluster_info: ClusterInfo {
                leader: info.leader.clone(),
                followers,
            },
            not_ready_followers,
            errors,
        }
    }

    async fn follower_is_ready(
        http_client: &reqwest::Client,
        node: &NodeInfo,
    ) -> Result<FollowerReadiness, reqwest::Error> {
        let url = format!("http://{}{}", node.address, READY_ENDPOINT_PATH);

        let response = http_client.get(url).send().await?;
        if response.status().is_success() {
            Ok(FollowerReadiness::Ready)
        } else {
            Ok(FollowerReadiness::NotReady(response.status()))
        }
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
