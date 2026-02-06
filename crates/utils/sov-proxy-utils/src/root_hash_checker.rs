use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::ClusterInfo;

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
}

impl ClusterRootHashChecker {
    /// Creates a new `ClusterRootHashChecker` with default timeouts.
    pub fn new() -> Result<Self> {
        let client = reqwest::ClientBuilder::new()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self { client })
    }

    /// Queries all nodes in the cluster for their latest slot's state root
    /// and checks whether they all agree.
    pub async fn check_root_hashes(&self, cluster_info: &ClusterInfo) -> Result<RootHashCheck> {
        let mut all_nodes: Vec<(&str, SocketAddr)> = Vec::new();

        if let Some(leader) = &cluster_info.leader {
            all_nodes.push((&leader.node_id, leader.address));
        }

        for follower in &cluster_info.followers {
            // Avoid duplicating the leader when it appears in followers (single-node cluster).
            if !all_nodes.iter().any(|(id, _)| *id == follower.node_id) {
                all_nodes.push((&follower.node_id, follower.address));
            }
        }

        let mut handles = Vec::with_capacity(all_nodes.len());
        for (node_id, address) in &all_nodes {
            let client = self.client.clone();
            let url = format!("http://{address}/ledger/slots/latest");
            let node_id = node_id.to_string();
            handles.push(tokio::spawn(async move {
                let result = client
                    .get(&url)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .context(format!("HTTP request to {url} failed"));

                let result = match result {
                    Ok(response) => response
                        .json::<SlotResponse>()
                        .await
                        .context("Failed to parse slot response"),
                    Err(e) => Err(e),
                };

                (node_id, result)
            }));
        }

        let mut node_results = HashMap::new();
        let mut failed_nodes = Vec::new();

        for handle in handles {
            let (node_id, result) = handle.await.context("Task join failed")?;
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
                Err(e) => {
                    tracing::warn!(node_id, error = %e, "Failed to fetch root hash from node");
                    failed_nodes.push((node_id, e.to_string()));
                }
            }
        }

        let consistent_root = if node_results.is_empty() {
            None
        } else {
            let mut roots = node_results.values().map(|(_, root)| root);
            let first = roots.next().unwrap();
            if roots.all(|r| r == first) {
                Some(first.clone())
            } else {
                None
            }
        };

        Ok(RootHashCheck {
            consistent_root,
            node_results,
            failed_nodes,
        })
    }
}
