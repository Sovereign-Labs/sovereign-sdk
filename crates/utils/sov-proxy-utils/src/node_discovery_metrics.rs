use sov_metrics::write_escaped_field_value;
use std::io::Write;

#[derive(Debug)]
pub(crate) struct ClusterUpdateMetric {
    pub current_leader: Option<String>,
    pub followers: Vec<String>,
    /// `true` if this emission reflects an actual membership or leader change;
    /// `false` if it is a periodic liveness re-emit of the unchanged state.
    pub cluster_changed: bool,
}

impl sov_metrics::Metric for ClusterUpdateMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_proxy_cluster_update"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // Leader and follower IDs are fields, not tags, because each distinct
        // tag value creates a new InfluxDB series and cluster membership can
        // change often enough to cause high-cardinality storage/query overhead.
        write!(buffer, "{} current_leader=\"", self.measurement_name(),)?;
        write_escaped_field_value(buffer, self.current_leader.as_deref().unwrap_or("none"))?;
        write!(buffer, "\",followers=\"")?;
        write_escaped_field_value(buffer, &format!("{:?}", self.followers))?;
        write!(buffer, "\",cluster_changed={}", self.cluster_changed)
    }
}

#[derive(Debug)]
pub(crate) struct ClusterUpdateFailureMetric {
    pub stage: &'static str,
}

impl sov_metrics::Metric for ClusterUpdateFailureMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_proxy_cluster_update_failure"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},stage={} count=1",
            self.measurement_name(),
            self.stage,
        )
    }
}
