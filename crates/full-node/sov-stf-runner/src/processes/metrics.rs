use std::io::Write;

use sov_metrics::Metric;

/// Metrics tracking the depth of the STF info channel between the state manager and the ZK prover.
/// Emitted after each `Sender::notify()` batch to detect backpressure.
#[derive(Debug)]
pub(crate) struct ZkStfInfoChannelMetrics {
    /// Number of items currently in the mpsc channel.
    pub channel_depth: usize,
    /// Maximum capacity of the channel.
    pub channel_capacity: usize,
}

impl Metric for ZkStfInfoChannelMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_zk_stf_info_channel"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} channel_depth={}i,channel_capacity={}i",
            self.measurement_name(),
            self.channel_depth,
            self.channel_capacity,
        )
    }
}

/// Metrics tracking the state of the ZK proof manager pipeline.
/// Emitted on every `process_stf_info()` call.
#[derive(Debug)]
pub(crate) struct ZkProofManagerMetrics {
    /// Difference between latest received slot and the first unproven height.
    /// Represents the total proving lag across the pipeline.
    pub proving_lag: u64,
    /// Number of blocks in the current aggregation batch.
    pub proofs_to_create: usize,
    /// The slot number of the most recently received state transition.
    pub slot_number: u64,
}

impl Metric for ZkProofManagerMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_zk_proof_manager"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} proving_lag={}i,proofs_to_create={}i,slot_number={}i",
            self.measurement_name(),
            self.proving_lag,
            self.proofs_to_create,
            self.slot_number,
        )
    }
}

/// Metrics emitted after each completed aggregated proof.
#[derive(Debug)]
pub(crate) struct ZkAggregatedProofMetrics {
    /// Wall-clock time for the full aggregated proof cycle, in milliseconds.
    pub aggregation_duration_ms: u128,
}

impl Metric for ZkAggregatedProofMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_zk_aggregated_proof"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} aggregation_duration_ms={}i",
            self.measurement_name(),
            self.aggregation_duration_ms,
        )
    }
}

/// Metrics tracking the number of in-flight proving tasks in the parallel prover service.
/// Emitted on every increment / decrement of the pending task counter.
#[derive(Debug)]
pub(crate) struct PendingProverTasksMetric {
    /// Number of proving tasks currently in flight.
    pub pending_tasks_count: usize,
}

impl Metric for PendingProverTasksMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_prover_service_pending_tasks"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} pending_tasks_count={}i",
            self.measurement_name(),
            self.pending_tasks_count,
        )
    }
}

/// Metrics for the network prover.
/// Emitted after each proof submission to the proving network.
#[derive(Debug)]
pub(crate) struct ZkNetworkProverMetrics {
    /// Time in milliseconds for submitting a proof request to the network.
    pub submit_duration_ms: u128,
}

impl Metric for ZkNetworkProverMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_zk_network_prover"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} submit_duration_ms={}i",
            self.measurement_name(),
            self.submit_duration_ms,
        )
    }
}
