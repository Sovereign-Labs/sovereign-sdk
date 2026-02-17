use crate::root_hash_checker::RootHashCheck;
use crate::root_hash_checker::RootHashConsistency;
use std::collections::BTreeSet;
use std::io::Write;

#[derive(Debug)]
pub struct RootHashCheckMetric {
    pub outcome: RootHashConsistency,
    pub slot_number: u64,
    pub nodes_ok: u64,
    pub nodes_failed: u64,
    pub unique_state_roots: u64,
}

impl RootHashCheckMetric {
    pub fn from_check(check: &RootHashCheck, outcome: RootHashConsistency) -> Self {
        let nodes_ok = check.node_results.len() as u64;
        let nodes_failed = check.failed_nodes.len() as u64;
        let unique_state_roots = check.node_results.values().collect::<BTreeSet<_>>().len() as u64;

        Self {
            outcome,
            slot_number: check.slot_number,
            nodes_ok,
            nodes_failed,
            unique_state_roots,
        }
    }

    fn outcome_tag(&self) -> &'static str {
        match self.outcome {
            RootHashConsistency::AllMatch => "all_match",
            RootHashConsistency::Mismatch => "mismatch",
            RootHashConsistency::NoData => "no_data",
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
            "{},outcome={} slot_number={},nodes_ok={},nodes_failed={},unique_state_roots={}",
            self.measurement_name(),
            self.outcome_tag(),
            self.slot_number,
            self.nodes_ok,
            self.nodes_failed,
            self.unique_state_roots,
        )
    }
}
