//! Defines utilities for collecting runtime metrics from inside a SP1 VM
use std::io::Write;

use sov_metrics::Metric;

/// The type of SP1 prover that generated a proof.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum ProverType {
    /// Local CPU prover.
    Cpu,
    /// Succinct proving network.
    Network,
}

impl ProverType {
    /// Returns the string representation for use in metrics serialization.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProverType::Cpu => "cpu",
            ProverType::Network => "network",
        }
    }
}

/// Metrics emitted when the SP1 proving network fulfills a proof request.
#[derive(Debug)]
pub(crate) struct SP1ProofFulfillmentMetrics {
    /// The type of prover emitting the metric.
    pub prover_type: ProverType,
    /// The hex-encoded proof request ID.
    pub request_id: String,
    /// Time from proof request creation to fulfillment, in seconds, as reported by the SP1
    /// network.
    pub fulfillment_duration_secs: u64,
}

impl Metric for SP1ProofFulfillmentMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_sp1_proof_fulfillment"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},prover_type={} request_id=\"{}\",fulfillment_duration_secs={}i",
            self.measurement_name(),
            self.prover_type.as_str(),
            self.request_id,
            self.fulfillment_duration_secs,
        )
    }
}
