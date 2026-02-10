use crate::node_discovery::ClusterInfo;
use crate::node_discovery::NodeInfo;
use anyhow::{Context, Result};
use sov_api_spec::types;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Default timeout for HTTP requests to node APIs.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Default timeout for establishing HTTP connections to node APIs.
const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
/// Minimum slot distance between consecutive root-hash checks.
const SLOT_QUERY_STEP: u64 = 3;
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

/// Result of one cluster-wide root-hash comparison at a specific slot.
#[derive(Debug, Default, Clone)]
pub struct RootHashCheck {
    /// Slot number requested for this root-hash check.
    pub slot_number: u64,
    /// Per-node results: node_id -> state_root.
    pub node_results: HashMap<String, String>,
    /// Nodes that failed to return a root hash `(node_id, error_message)`.
    pub failed_nodes: Vec<(String, String)>,
}

impl RootHashCheck {
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
}

/// Task returned when spawning the root-hash checker.
pub struct ClusterRootHashCheckerTask {
    /// Subscription receiver for root-hash check results.
    pub receiver: watch::Receiver<RootHashCheck>,
    /// Join handle of the background root-hash checker task.
    pub(crate) handle: JoinHandle<()>,
}

impl ClusterRootHashCheckerTask {
    pub fn abort(&self) {
        self.handle.abort();
    }
}

/// Periodically checks that all cluster nodes agree on the same finalized root hash.
pub struct ClusterRootHashChecker {
    cluster_info_receiver: watch::Receiver<ClusterInfo>,
    http_client: reqwest::Client,
}

impl ClusterRootHashChecker {
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

    /// Spawns a task that periodically queries all nodes and verifies root-hash consistency.
    pub fn spawn(self) -> ClusterRootHashCheckerTask {
        let (sender, receiver) = watch::channel(RootHashCheck::default());

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

                let root_hash_check = self
                    .get_slot_root_hashes_in_parallel(nodes, slot_hash, slot_number)
                    .await;

                if !root_hash_check.failed_nodes.is_empty() {
                    tracing::warn!(
                        slot_number = root_hash_check.slot_number,
                        failed_nodes = ?root_hash_check.failed_nodes,
                        "Failed to fetch root hash from some cluster nodes, will compare root hash from the remaining"
                    );
                }

                // TODO: Add a metric for root hash consistency check.
                match root_hash_check.check_consistency() {
                    RootHashConsistency::AllMatch => {
                        tracing::info!(
                            slot_number = root_hash_check.slot_number,
                            "Cluster root hashes are consistent"
                        );
                    }
                    RootHashConsistency::Mismatch => {
                        tracing::error!(
                            slot_number = root_hash_check.slot_number,
                            root_hashes = ?root_hash_check.node_results,
                            "Cluster root hash mismatch detected"
                        );
                    }
                    RootHashConsistency::NoData => {
                        tracing::warn!(
                            slot_number = root_hash_check.slot_number,
                            failed_nodes = ?root_hash_check.failed_nodes,
                            "No root hashes collected from cluster nodes"
                        );
                    }
                }

                // Update the last checked slot number no matter what is the result of root_hash_check.
                last_checked_slot_number = Some(root_hash_check.slot_number);
                let _ = sender.send(root_hash_check);
            }
        });

        ClusterRootHashCheckerTask { receiver, handle }
    }

    async fn get_finalized_slot_for_comparison(
        &self,
        cluster_info: &ClusterInfo,
    ) -> Result<types::Slot> {
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
    ) -> Result<types::Slot> {
        let url = format!("http://{address}/ledger/slots/{slot_id}");
        let response = http_client.get(&url).send().await?;
        let response = response.error_for_status()?;
        let slot = response.json::<types::Slot>().await?;
        Ok(slot)
    }

    async fn get_finalized_slot(&self, node: &NodeInfo) -> Result<types::Slot> {
        Self::get_slot(&self.http_client, "finalized", &node.address)
            .await
            .with_context(|| {
                format!(
                    "Failed to get finalized slot from node {} at {}",
                    node.node_id, node.address
                )
            })
    }

    async fn get_slot_root_hashes_in_parallel(
        &self,
        nodes: Vec<NodeInfo>,
        slot_hash: &str,
        slot_number: u64,
    ) -> RootHashCheck {
        let mut query_tasks = Vec::with_capacity(nodes.len());

        for node in nodes {
            let http_client = self.http_client.clone();
            let node_id = node.node_id;
            let node_id_for_task = node_id.clone();
            let node_address = node.address;
            let slot_hash = slot_hash.to_owned();

            let task = tokio::spawn(async move {
                let slot = Self::get_slot(&http_client, &slot_hash, &node_address)
                    .await
                    .map_err(|err| {
                        format!("Failed to get slot {slot_number} ({slot_hash}) from node {node_id_for_task} at {node_address}, error: {err}")
                    })?;

                Ok(slot.state_root.to_string())
            });
            query_tasks.push((node_id, task));
        }

        let mut node_results = HashMap::new();
        let mut failed_nodes = Vec::new();
        for (node_id, task) in query_tasks {
            match task.await {
                Ok(Ok(state_root)) => {
                    node_results.insert(node_id, state_root);
                }
                Ok(Err(error_message)) => {
                    failed_nodes.push((node_id, error_message));
                }
                Err(join_error) => {
                    failed_nodes.push((
                        node_id,
                        format!("Root-hash query task failed to join: {join_error}"),
                    ));
                }
            }
        }

        RootHashCheck {
            slot_number,
            node_results,
            failed_nodes,
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
        let mut node_results = HashMap::new();
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
        let mut node_results = HashMap::new();
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
            node_results: HashMap::new(),
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
        assert!(ClusterRootHashChecker::should_query_slot(None, 100));
        assert!(!ClusterRootHashChecker::should_query_slot(Some(100), 104));
        assert!(ClusterRootHashChecker::should_query_slot(Some(100), 110));
    }

    #[test]
    fn should_query_slot_when_slot_moves_backwards() {
        assert!(ClusterRootHashChecker::should_query_slot(Some(100), 99));
    }
}
