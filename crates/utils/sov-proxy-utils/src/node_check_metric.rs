use crate::node_checker::RootHashCheck;
use crate::LatestHeightCheckStats;
use std::collections::BTreeSet;
use std::io::Write;

#[derive(Debug)]
pub struct LatestHeightCheckMetric {
    pub stats: LatestHeightCheckStats,
}

impl sov_metrics::Metric for LatestHeightCheckMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_proxy_latest_height_check"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let stats = &self.stats;
        write!(
            buffer,
            "{} nodes_ok={},nodes_failed={},height_diff={}",
            self.measurement_name(),
            stats.nodes_ok,
            stats.nodes_failed,
            stats.height_diff,
        )
    }
}

#[derive(Debug)]
pub struct RootHashCheckMetric {
    pub slot_number: u64,
    pub nodes_ok: u64,
    pub nodes_failed: u64,
    pub unique_state_roots: u64,
}

impl RootHashCheckMetric {
    pub(crate) fn from_check(check: &RootHashCheck) -> Self {
        let nodes_ok = check.node_results.len() as u64;
        let nodes_failed = check.failed_nodes.len() as u64;
        let unique_state_roots = check.node_results.values().collect::<BTreeSet<_>>().len() as u64;

        Self {
            slot_number: check.slot_number,
            nodes_ok,
            nodes_failed,
            unique_state_roots,
        }
    }
}

impl sov_metrics::Metric for RootHashCheckMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_proxy_root_hash_check"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} slot_number={},nodes_ok={},nodes_failed={},unique_state_roots={}",
            self.measurement_name(),
            self.slot_number,
            self.nodes_ok,
            self.nodes_failed,
            self.unique_state_roots,
        )
    }
}
