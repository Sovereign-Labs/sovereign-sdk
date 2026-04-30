use std::io::Write;

use sov_metrics::Metric;

#[derive(Debug)]
pub(crate) struct LatestVerifiedProofMetric {
    pub final_slot_number: u64,
    pub execution_context: &'static str,
}

impl Metric for LatestVerifiedProofMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_prover_incentives_latest_verified_proof"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},context={} final_slot_number={}",
            self.measurement_name(),
            self.execution_context,
            self.final_slot_number
        )
    }
}
