use std::io::Write;

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
