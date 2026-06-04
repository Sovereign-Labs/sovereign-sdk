use sov_metrics::write_escaped_field_value;
use std::io::Write;

#[derive(Debug)]
pub(crate) struct ClusterUpdateMetric {
    pub current_leader: Option<String>,
    /// All followers registered in the database, regardless of readiness.
    pub followers: Vec<String>,
    /// `true` if this emission reflects an actual membership or leader change;
    /// `false` if it is a periodic liveness re-emit of the unchanged state.
    pub cluster_changed: bool,
    /// Followers that reported themselves ready and are advertised to the proxy.
    pub advertised_followers: Vec<String>,
    /// `true` if the advertised (ready) set or leader changed in this emission.
    pub advertised_changed: bool,
    /// Followers registered but not currently advertised because they are not
    /// ready.
    pub not_ready_followers: Vec<String>,
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
        write!(buffer, "\",cluster_changed={}", self.cluster_changed)?;
        write!(buffer, ",advertised_followers=\"")?;
        write_escaped_field_value(buffer, &format!("{:?}", self.advertised_followers))?;
        write!(
            buffer,
            "\",advertised_changed={}",
            self.advertised_changed
        )?;
        write!(buffer, ",not_ready_followers=\"")?;
        write_escaped_field_value(buffer, &format!("{:?}", self.not_ready_followers))?;
        write!(buffer, "\"")
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
