use crate::ClusterInfo;
use anyhow::{Context, Result};
use futures::future::join_all;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::watch;

/// Response from the `/ledger/slots/latest` endpoint.
/// Only the fields we need are deserialized.
#[derive(serde::Deserialize)]
struct SlotResponse {
    number: u64,
    state_root: String,
}

/// Result of checking root hashes across cluster nodes.
#[derive(Debug)]
pub struct RootHashCheck {
    /// The state root hash that all responding nodes agree on, if consistent.
    pub consistent_root: Option<String>,
    /// Per-node results: node_id -> (slot_number, state_root).
    pub node_results: HashMap<String, (u64, String)>,
    /// Nodes that failed to respond.
    pub failed_nodes: Vec<(String, String)>,
}

impl RootHashCheck {
    /// Returns true if all responding nodes have the same root hash.
    pub fn is_consistent(&self) -> bool {
        self.consistent_root.is_some() && self.failed_nodes.is_empty()
    }
}

/// Client for checking root hash consistency across cluster nodes.
pub struct ClusterRootHashChecker {
    client: reqwest::Client,
    receiver: watch::Receiver<ClusterInfo>,
}

impl ClusterRootHashChecker {
    /// Creates a new `ClusterRootHashChecker` with default timeouts.
    pub fn new(receiver: watch::Receiver<ClusterInfo>) -> Result<Self> {
        let client = reqwest::ClientBuilder::new()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self { client, receiver })
    }

    fn cluster_nodes(cluster_info: &ClusterInfo) -> Vec<(String, SocketAddr)> {
        let mut nodes = Vec::new();

        if let Some(leader) = &cluster_info.leader {
            nodes.push((leader.node_id.clone(), leader.address));
        }

        for follower in &cluster_info.followers {
            // Avoid duplicating the leader when it appears in followers (single-node cluster).
            if !nodes
                .iter()
                .any(|(node_id, _)| node_id == &follower.node_id)
            {
                nodes.push((follower.node_id.clone(), follower.address));
            }
        }

        nodes
    }

    async fn fetch_latest_slot(
        client: reqwest::Client,
        address: SocketAddr,
    ) -> Result<SlotResponse> {
        let url = format!("http://{address}/ledger/slots/latest");
        let response = client
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .with_context(|| format!("HTTP request to {url} failed"))?;

        response
            .json::<SlotResponse>()
            .await
            .context("Failed to parse slot response")
    }

    fn consistent_root(
        cluster_info: &ClusterInfo,
        node_results: &HashMap<String, (u64, String)>,
    ) -> Option<String> {
        let reference_node_id = cluster_info
            .leader
            .as_ref()
            .map(|leader| leader.node_id.as_str())
            .or_else(|| {
                cluster_info
                    .followers
                    .first()
                    .map(|follower| follower.node_id.as_str())
            })?;

        let (_, reference_root) = node_results.get(reference_node_id)?;
        if node_results
            .values()
            .all(|(_, other_root)| other_root == reference_root)
        {
            Some(reference_root.clone())
        } else {
            None
        }
    }

    /// Queries all nodes in the cluster for their latest slot's state root
    /// and checks whether they all agree.
    pub async fn check_root_hashes(&self) -> RootHashCheck {
        let cluster_info = self.receiver.borrow().clone();
        let fetches = Self::cluster_nodes(&cluster_info)
            .into_iter()
            .map(|(node_id, address)| {
                let client = self.client.clone();
                async move {
                    let result = Self::fetch_latest_slot(client, address).await;
                    (node_id, result)
                }
            });

        let fetches = join_all(fetches).await;

        let mut node_results = HashMap::new();
        let mut failed_nodes = Vec::new();

        for (node_id, result) in fetches {
            match result {
                Ok(slot) => {
                    tracing::debug!(
                        node_id,
                        slot_number = slot.number,
                        state_root = slot.state_root,
                        "Fetched root hash from node"
                    );
                    node_results.insert(node_id, (slot.number, slot.state_root));
                }
                Err(error) => {
                    tracing::warn!(node_id, error = %error, "Failed to fetch root hash from node");
                    failed_nodes.push((node_id, error.to_string()));
                }
            }
        }

        let consistent_root = Self::consistent_root(&cluster_info, &node_results);

        RootHashCheck {
            consistent_root,
            node_results,
            failed_nodes,
        }
    }

    /// Periodically checks root hash consistency every 10 seconds.
    ///
    /// Logs warnings when nodes disagree and errors when checks fail.
    /// Runs indefinitely.
    pub async fn run(&self) -> ! {
        let interval = Duration::from_secs(10);

        loop {
            let check = self.check_root_hashes().await;
            if check.is_consistent() {
                tracing::debug!("Root hash check passed: all nodes consistent");
            } else {
                tracing::warn!(
                ?check.node_results,
                ?check.failed_nodes,
                "Root hash inconsistency detected across cluster nodes"
                );
            }

            tokio::time::sleep(interval).await;
        }
    }
}
