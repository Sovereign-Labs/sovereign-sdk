use sov_metrics::Metric;
use std::io::Write;

#[derive(Debug)]
pub(crate) struct EvmTxMetrics {
    pub total_time: std::time::Duration,
}

impl Metric for EvmTxMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_evm_tx"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} total_time={}",
            self.measurement_name(),
            self.total_time.as_micros(),
        )
    }
}
