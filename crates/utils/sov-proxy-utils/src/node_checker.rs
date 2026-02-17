use crate::node_check_metric::LatestHeightCheckMetric;
use crate::node_check_metric::RootHashCheckMetric;
use crate::node_discovery::ClusterInfo;
use crate::node_discovery::NodeInfo;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinError;
use tokio::task::JoinHandle;

/// Subset of slot fields returned by ledger APIs used for root-hash checking.
#[derive(Debug, Clone, serde::Deserialize)]
struct Slot {
    number: u64,
    hash: String,
    state_root: String,
}

/// Response payload returned by the chain-state current-heights endpoint.
#[derive(Debug, serde::Deserialize)]
struct CurrentHeightsResponse {
    value: (u64, u64),
}

/// Default timeout for HTTP requests to node APIs.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Default timeout for establishing HTTP connections to node APIs.
const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
/// Minimum slot distance between queries.
const SLOT_QUERY_STEP: u64 = 1;
/// Period between iterations of the background root-hash checker task.
const DEFAULT_ROOT_HASH_CHECK_INTERVAL: Duration = Duration::from_secs(5);

/// Result of evaluating root-hash consistency for one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootHashConsistency {
    /// All collected root hashes are identical.
    AllMatch,
    /// At least two collected root hashes differ.
    Mismatch,
    /// No node returned a root hash.
    NoData,
}

#[derive(Debug, Default, Clone)]
pub struct NodeCheckOutput {
    /// Root-hash check results for the queried reference slot.
    pub root_hash_check: RootHashCheck,
    /// Latest rollup-height check results collected from all nodes.
    pub latest_height_check: LatestHeightCheck,
}

#[derive(Debug)]
pub struct LatestHeightCheckStats {
    pub nodes_ok: u64,
    pub nodes_failed: u64,
    pub spread: Option<NodeHeightSpread>,
    pub height_diff: u64,
}

impl LatestHeightCheckStats {
    fn log_check_height_stats(&self) {
        if self.nodes_failed > 0 {
            tracing::warn!(
                nodes_failed = self.nodes_failed,
                "Failed to fetch latest height from some cluster nodes, will compare latest heights from the remaining"
            );
        }

        if self.nodes_ok == 0 {
            tracing::warn!(
                nodes_failed = self.nodes_failed,
                "No latest heights collected from cluster nodes"
            );
            return;
        }

        tracing::debug!(
            ?self.nodes_ok,
            ?self.nodes_failed,
            ?self.height_diff,
            ?self.spread,
            "Cluster latest heights."
        );
    }
}

/// Result of one cluster-wide latest-height collection round.
#[derive(Debug, Default, Clone)]
pub struct LatestHeightCheck {
    /// Per-node results: node_id -> current rollup height.
    pub(crate) node_results: BTreeMap<String, u64>,
    /// Nodes that failed to return latest height `(node_id, error_message)`.
    pub(crate) failed_nodes: Vec<(String, String)>,
}

/// Tracks the lowest and highest observed rollup height across responding nodes.
#[derive(Debug, Clone)]
pub struct NodeHeightSpread {
    // `(node_id, height)` tuple for the lowest observed height.
    min: (String, u64),
    // `(node_id, height)` tuple for the highest observed height.
    max: (String, u64),
}

impl NodeHeightSpread {
    fn update_max(&mut self, node_id: String, height: u64) {
        if height > self.max.1 {
            self.max = (node_id, height);
        }
    }

    fn update_min(&mut self, node_id: String, height: u64) {
        if height < self.min.1 {
            self.min = (node_id, height);
        }
    }

    fn diff(&self) -> u64 {
        self.max.1.checked_sub(self.min.1).unwrap_or_else(|| {
            panic!(
                "NodeHeightSpread: Impossible the min height {} > max height {}",
                self.min.1, self.max.1,
            )
        })
    }
}

impl LatestHeightCheck {
    pub(crate) fn stats(&self) -> LatestHeightCheckStats {
        let nodes_ok = self.node_results.len() as u64;
        let nodes_failed = self.failed_nodes.len() as u64;

        let mut spread: Option<NodeHeightSpread> = None;

        for (node_id, &height) in &self.node_results {
            match spread.as_mut() {
                Some(spread) => {
                    spread.update_min(node_id.clone(), height);
                    spread.update_max(node_id.clone(), height);
                }
                None => {
                    spread = Some(NodeHeightSpread {
                        min: (node_id.clone(), height),
                        max: (node_id.clone(), height),
                    });
                }
            }
        }

        let height_diff = spread.as_ref().map(|e| e.diff()).unwrap_or_default();

        LatestHeightCheckStats {
            nodes_ok,
            nodes_failed,
            spread,
            height_diff,
        }
    }
}

impl LatestHeightCheck {
    /// Spawns a task that queries one node for `/modules/chain-state/state/current-heights`
    /// and returns the rollup height component.
    fn spawn(
        http_client: reqwest::Client,
        node_id: String,
        node_address: SocketAddr,
    ) -> JoinHandle<Result<u64, String>> {
        tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            let height = NodeChecker::get_latest_height(&http_client, &node_address)
                .await
                .map_err(|err| {
                    format!(
                        "Failed to get latest heights from node {node_id} at {node_address}, error: {err}"
                    )
                });

            let elapsed = start.elapsed();

            tracing::trace!(
                %node_id,
                ?elapsed,
                "Duration of the rollup height query");

            height
        })
    }
}

/// Result of one cluster-wide root-hash comparison at a specific slot.
#[derive(Debug, Default, Clone)]
pub struct RootHashCheck {
    /// Slot number requested for this root-hash check.
    pub(crate) slot_number: u64,
    /// Per-node results: node_id -> state_root.
    pub(crate) node_results: BTreeMap<String, String>,
    /// Nodes that failed to return a root hash `(node_id, error_message)`.
    pub(crate) failed_nodes: Vec<(String, String)>,
}

impl RootHashCheck {
    fn spawn(
        http_client: reqwest::Client,
        node_id: String,
        node_address: SocketAddr,
        slot_hash: String,
        slot_number: u64,
    ) -> JoinHandle<Result<String, String>> {
        tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            let state_root = NodeChecker::get_slot(&http_client, &slot_hash, &node_address)
                .await.map(|slot|slot.state_root)
                .map_err(|err| {
                    format!(
                        "Failed to get slot {slot_number} ({slot_hash}) from node {node_id} at {node_address}, error: {err}"
                    )
                });

            let elapsed = start.elapsed();

            tracing::trace!(
                %node_id,
                ?elapsed,
                "Duration of the root hash query");

            state_root
        })
    }

    /// Evaluates whether the observed root hashes are consistent.
    /// Returns `AllMatch` even when only one node responded.
    pub fn check_consistency(&self) -> RootHashConsistency {
        let mut values = self.node_results.values();
        let Some(first) = values.next() else {
            return RootHashConsistency::NoData;
        };

        if values.all(|root_hash| root_hash == first) {
            RootHashConsistency::AllMatch
        } else {
            RootHashConsistency::Mismatch
        }
    }

    fn log_consistency(&self) {
        let consistency = self.check_consistency();

        if !self.failed_nodes.is_empty() {
            tracing::warn!(
                slot_number = self.slot_number,
                failed_nodes = ?self.failed_nodes,
                "Failed to fetch root hash from some cluster nodes, will compare root hash from the remaining"
            );
        }

        match consistency {
            RootHashConsistency::AllMatch => {
                tracing::debug!(
                    slot_number = self.slot_number,
                    "Cluster root hashes are consistent"
                );
            }
            RootHashConsistency::Mismatch => {
                tracing::error!(
                    slot_number = self.slot_number,
                    root_hashes = ?self.node_results,
                    "Cluster root hash mismatch detected"
                );
            }
            RootHashConsistency::NoData => {
                tracing::warn!(
                    slot_number = self.slot_number,
                    failed_nodes = ?self.failed_nodes,
                    "No root hashes collected from cluster nodes"
                );
            }
        }
    }
}

/// Task returned when spawning the node checker loop.
pub struct NodeCheckerTask {
    /// Subscription receiver for root-hash check results.
    pub receiver: watch::Receiver<NodeCheckOutput>,
    /// Join handle of the background checker task.
    pub(crate) handle: JoinHandle<()>,
}

impl NodeCheckerTask {
    pub fn abort(&self) {
        self.handle.abort();
    }
}

/// Periodically checks cluster-node root-hash consistency and collects latest heights.
pub struct NodeChecker {
    cluster_info_receiver: watch::Receiver<ClusterInfo>,
    http_client: reqwest::Client,
}

impl NodeChecker {
    /// Creates a checker that reads cluster members from `cluster_info_receiver`.
    pub fn new(cluster_info_receiver: watch::Receiver<ClusterInfo>) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .connect_timeout(DEFAULT_CONNECTION_TIMEOUT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .build()?;

        Ok(Self {
            cluster_info_receiver,
            http_client,
        })
    }

    /// Spawns a task that periodically queries all nodes for:
    /// - root hash at a reference finalized slot, and
    /// - latest rollup height from chain-state.
    pub fn spawn(self) -> NodeCheckerTask {
        let (sender, receiver) = watch::channel(NodeCheckOutput::default());

        let handle: JoinHandle<()> = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(DEFAULT_ROOT_HASH_CHECK_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut last_checked_slot_number = None;

            loop {
                ticker.tick().await;

                let cluster_info = self.cluster_info_receiver.borrow().clone();
                let reference_slot = match self
                    .get_finalized_slot_for_comparison(&cluster_info)
                    .await
                {
                    Ok(slot) => slot,
                    Err(error) => {
                        tracing::warn!(?error, "Failed to get comparison slot for root-hash check");
                        continue;
                    }
                };

                let slot_number = reference_slot.number;
                if !Self::should_query_slot(last_checked_slot_number, slot_number) {
                    tracing::trace!(
                        ?last_checked_slot_number,
                        slot_number,
                        slot_query_step = SLOT_QUERY_STEP,
                        "Skipping root-hash check because slot has not advanced enough"
                    );
                    continue;
                }

                let nodes = flatten_cluster_nodes(&cluster_info);
                let slot_hash = &reference_slot.hash;

                let node_check_output = self
                    .query_nodes_in_parallel(nodes, slot_hash, slot_number)
                    .await;

                let height_check_stats = node_check_output.latest_height_check.stats();

                height_check_stats.log_check_height_stats();
                node_check_output.root_hash_check.log_consistency();

                sov_metrics::track_metrics(|tracker| {
                    tracker.submit(RootHashCheckMetric::from_check(
                        &node_check_output.root_hash_check,
                    ));
                    tracker.submit(LatestHeightCheckMetric {
                        stats: height_check_stats,
                    });
                });

                // Update the last checked slot number no matter what is the result of root_hash_check.
                last_checked_slot_number = Some(node_check_output.root_hash_check.slot_number);
                let _ = sender.send(node_check_output);
            }
        });

        NodeCheckerTask { receiver, handle }
    }

    async fn get_finalized_slot_for_comparison(&self, cluster_info: &ClusterInfo) -> Result<Slot> {
        let reference_node = cluster_info
            .leader
            .as_ref()
            .or_else(|| cluster_info.followers.values().next())
            .context("Cannot check root hash consistency because cluster has no nodes")?;

        let reference_finalized_slot = self.get_finalized_slot(reference_node).await?;

        if cluster_info.leader.is_none() {
            tracing::trace!(
                reference_node_id = %reference_node.node_id,
                finalized_slot_number = reference_finalized_slot.number,
                "No leader present; using first follower as root-hash reference node"
            );
        }

        Ok(reference_finalized_slot)
    }

    fn should_query_slot(last_checked_slot_number: Option<u64>, slot_number: u64) -> bool {
        let Some(last_checked_slot_number) = last_checked_slot_number else {
            return true;
        };

        if slot_number < last_checked_slot_number {
            // The reference slot can decrease when leadership changes.
            return true;
        }

        slot_number - last_checked_slot_number >= SLOT_QUERY_STEP
    }

    async fn get_slot(
        http_client: &reqwest::Client,
        slot_id: &str,
        address: &SocketAddr,
    ) -> Result<Slot> {
        let url = format!("http://{address}/ledger/slots/{slot_id}");
        let response = http_client.get(&url).send().await?;
        let response = response.error_for_status()?;
        let slot = response.json::<Slot>().await?;
        Ok(slot)
    }

    async fn get_finalized_slot(&self, node: &NodeInfo) -> Result<Slot> {
        Self::get_slot(&self.http_client, "finalized", &node.address)
            .await
            .with_context(|| {
                format!(
                    "Failed to get finalized slot from node {} at {}",
                    node.node_id, node.address
                )
            })
    }

    async fn get_latest_height(http_client: &reqwest::Client, address: &SocketAddr) -> Result<u64> {
        let url = format!("http://{address}/modules/chain-state/state/current-heights");
        let response = http_client.get(&url).send().await?;
        let response = response.error_for_status()?;
        let heights = response.json::<CurrentHeightsResponse>().await?;
        Ok(heights.value.0)
    }

    async fn query_nodes_in_parallel(
        &self,
        nodes: Vec<NodeInfo>,
        slot_hash: &str,
        slot_number: u64,
    ) -> NodeCheckOutput {
        let mut query_tasks = Vec::with_capacity(nodes.len());

        for node in nodes {
            let node_id = node.node_id;
            let node_address = node.address;
            let slot_hash = slot_hash.to_owned();

            let root_hash_check_task = RootHashCheck::spawn(
                self.http_client.clone(),
                node_id.clone(),
                node_address,
                slot_hash,
                slot_number,
            );

            let latest_height_check_task =
                LatestHeightCheck::spawn(self.http_client.clone(), node_id.clone(), node_address);

            query_tasks.push((node_id, root_hash_check_task, latest_height_check_task));
        }

        let mut root_hash_check = RootHashCheck {
            slot_number,
            node_results: BTreeMap::new(),
            failed_nodes: Vec::new(),
        };

        let mut latest_height_check = LatestHeightCheck {
            node_results: BTreeMap::new(),
            failed_nodes: Vec::new(),
        };

        for (node_id, root_hash_check_task, latest_height_check_task) in query_tasks {
            record_task_result(
                node_id.clone(),
                root_hash_check_task.await,
                &mut root_hash_check.node_results,
                &mut root_hash_check.failed_nodes,
                "Root-hash query task failed to join",
            );

            record_task_result(
                node_id,
                latest_height_check_task.await,
                &mut latest_height_check.node_results,
                &mut latest_height_check.failed_nodes,
                "Node height query task failed to join",
            );
        }

        NodeCheckOutput {
            root_hash_check,
            latest_height_check,
        }
    }
}

fn record_task_result<T>(
    node_id: String,
    task_result: Result<Result<T, String>, JoinError>,
    node_results: &mut BTreeMap<String, T>,
    failed_nodes: &mut Vec<(String, String)>,
    join_error_label: &str,
) {
    match task_result {
        Ok(Ok(value)) => {
            node_results.insert(node_id, value);
        }
        Ok(Err(error_message)) => {
            failed_nodes.push((node_id, error_message));
        }
        Err(join_error) => {
            failed_nodes.push((node_id, format!("{join_error_label}: {join_error}")));
        }
    }
}

fn flatten_cluster_nodes(cluster_info: &ClusterInfo) -> Vec<NodeInfo> {
    let mut nodes = Vec::with_capacity(
        cluster_info.followers.len() + usize::from(cluster_info.leader.is_some()),
    );

    if let Some(leader) = &cluster_info.leader {
        nodes.push(leader.clone());
    }

    nodes.extend(cluster_info.followers.values().cloned());
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_discovery::OffsetDateTime;
    use std::collections::BTreeMap;

    fn test_node(node_id: &str, port: u16) -> NodeInfo {
        NodeInfo {
            node_id: node_id.to_owned(),
            address: format!("127.0.0.1:{port}")
                .parse()
                .expect("valid socket addr"),
            last_updated: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn all_match_returns_true_when_all_hashes_are_equal() {
        let mut node_results = BTreeMap::new();
        node_results.insert(
            "node1".to_string(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        );
        node_results.insert(
            "node2".to_string(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        );

        let snapshot = RootHashCheck {
            slot_number: 10,
            node_results,
            failed_nodes: vec![],
        };

        assert_eq!(snapshot.check_consistency(), RootHashConsistency::AllMatch);
    }

    #[test]
    fn all_match_returns_false_when_hashes_differ() {
        let mut node_results = BTreeMap::new();
        node_results.insert(
            "node1".to_string(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        );
        node_results.insert(
            "node2".to_string(),
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        );

        let snapshot = RootHashCheck {
            slot_number: 11,
            node_results,
            failed_nodes: vec![],
        };

        assert_eq!(snapshot.check_consistency(), RootHashConsistency::Mismatch);
    }

    #[test]
    fn all_match_returns_no_data_when_no_hashes_are_collected() {
        let snapshot = RootHashCheck {
            slot_number: 12,
            node_results: BTreeMap::new(),
            failed_nodes: vec![],
        };

        assert_eq!(snapshot.check_consistency(), RootHashConsistency::NoData);
    }

    #[test]
    fn flatten_cluster_nodes_returns_leader_and_followers() {
        let leader = test_node("leader", 1111);
        let mut followers = BTreeMap::new();
        followers.insert("follower-a".to_string(), test_node("follower-a", 2222));
        followers.insert("follower-b".to_string(), test_node("follower-b", 3333));
        let cluster_info = ClusterInfo {
            leader: Some(leader),
            followers,
        };

        let nodes = flatten_cluster_nodes(&cluster_info);

        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].node_id, "leader");
        assert_eq!(nodes[1].node_id, "follower-a");
        assert_eq!(nodes[2].node_id, "follower-b");
    }

    #[test]
    fn should_query_slot_only_after_slot_step() {
        assert!(NodeChecker::should_query_slot(None, 100));
        assert!(!NodeChecker::should_query_slot(Some(100), 100));
        assert!(NodeChecker::should_query_slot(Some(100), 110));
    }

    #[test]
    fn should_query_slot_when_slot_moves_backwards() {
        assert!(NodeChecker::should_query_slot(Some(100), 99));
    }

    #[test]
    fn node_height_spread_update_min_and_max_and_diff() {
        let mut spread = NodeHeightSpread {
            min: ("node-start".to_string(), 10),
            max: ("node-start".to_string(), 10),
        };

        spread.update_min("node-low".to_string(), 3);
        spread.update_max("node-high".to_string(), 25);

        assert_eq!(spread.min, ("node-low".to_string(), 3));
        assert_eq!(spread.max, ("node-high".to_string(), 25));
        assert_eq!(spread.diff(), 22);
    }

    #[test]
    fn node_height_spread_ignore_equal_heights() {
        let mut spread = NodeHeightSpread {
            min: ("node-a".to_string(), 10),
            max: ("node-b".to_string(), 20),
        };

        spread.update_min("node-c".to_string(), 10);
        spread.update_max("node-d".to_string(), 20);

        assert_eq!(spread.min, ("node-a".to_string(), 10));
        assert_eq!(spread.max, ("node-b".to_string(), 20));
    }

    #[test]
    fn latest_height_stats_sets_spread_and_diff() {
        let mut node_results = BTreeMap::new();
        node_results.insert("node-a".to_string(), 15);
        node_results.insert("node-b".to_string(), 9);
        node_results.insert("node-c".to_string(), 28);

        let check = LatestHeightCheck {
            node_results,
            failed_nodes: vec![("node-x".to_string(), "timeout".to_string())],
        };
        let stats = check.stats();

        assert_eq!(stats.nodes_ok, 3);
        assert_eq!(stats.nodes_failed, 1);
        assert_eq!(stats.height_diff, 19);
        assert_eq!(
            stats.spread.as_ref().map(|e| e.min.clone()),
            Some(("node-b".to_string(), 9))
        );
        assert_eq!(
            stats.spread.as_ref().map(|e| e.max.clone()),
            Some(("node-c".to_string(), 28))
        );
    }

    #[test]
    fn latest_height_stats_with_no_results_has_no_spread() {
        let check = LatestHeightCheck {
            node_results: BTreeMap::new(),
            failed_nodes: vec![("node-x".to_string(), "timeout".to_string())],
        };
        let stats = check.stats();

        assert_eq!(stats.nodes_ok, 0);
        assert_eq!(stats.nodes_failed, 1);
        assert_eq!(stats.height_diff, 0);
        assert!(stats.spread.is_none());
    }
}
