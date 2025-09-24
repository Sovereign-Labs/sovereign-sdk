use sov_metrics::Metric;
use std::{io::Write, time::Duration};

#[derive(Debug)]
pub(crate) struct EvmTxMetrics {
    pub total_time: Duration,
    pub fetch_state_time: Duration,
    pub get_db_time: Duration,
    pub execution_time: Duration,
    pub state_commit_time: Duration,
    pub receipt_time: Duration,
    pub set_accessory_state_time: Duration,
    pub set_state_time: Duration,
}

impl Metric for EvmTxMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_evm_tx"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let fields: &[(&str, Duration)] = &[
            ("total_time", self.total_time),
            ("fetch_prestate_time", self.fetch_state_time),
            ("get_db_time", self.get_db_time),
            ("execution_time", self.execution_time),
            ("state_commit_time", self.state_commit_time),
            ("receipt_time", self.receipt_time),
            ("set_state_time", self.set_state_time),
            ("set_accessory_state_time", self.set_accessory_state_time),
        ];
        write!(buffer, "{}", self.measurement_name())?;
        for (i, (name, val)) in fields.iter().enumerate() {
            let sep = if i == 0 { ' ' } else { ',' };
            write!(buffer, "{sep}{name}={}", val.as_micros())?;
        }
        Ok(())
    }
}
