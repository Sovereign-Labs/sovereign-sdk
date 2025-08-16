use sov_rollup_interface::common::HexHash;

use crate::MaybeTimer;
#[cfg(feature = "native")]
use crate::Metric;

/// Metrics for a single state access.
#[derive(Debug)]
pub struct StateAccessMetric {
    #[allow(missing_docs)]
    pub op: &'static str,
    #[allow(missing_docs)]
    pub key_size: usize,
    #[allow(missing_docs)]
    pub storage_read_size: Option<u32>,
    #[allow(missing_docs)]
    pub duration: MaybeTimer,
}

impl StateAccessMetric {
    /// Creates a new state access metric.
    pub fn new(op: &'static str, key_size: usize) -> Self {
        Self {
            op,
            key_size,
            storage_read_size: None,
            duration: MaybeTimer::started(),
        }
    }
}

#[cfg(feature = "native")]
fn summarize(
    metrics: &[StateAccessMetric],
    prefix: &str,
    target: &mut Vec<u8>,
) -> std::io::Result<()> {
    use std::io::Write;
    let total_reads = metrics.len();
    let mut cache_misses = 0;
    let mut cache_miss_bytes = 0;
    let mut slowest_read = std::time::Duration::from_millis(0);
    let mut slowest_read_name = "none";
    let mut slowest_read_key_size = 0;
    let mut slowest_read_storage_read_size = 0;
    for metric in metrics {
        cache_misses += metric.storage_read_size.is_some() as usize;
        cache_miss_bytes += metric.storage_read_size.unwrap_or(0);
        if metric.duration.elapsed() > slowest_read {
            slowest_read = metric.duration.elapsed();
            slowest_read_name = metric.op;
            slowest_read_key_size = metric.key_size;
            slowest_read_storage_read_size = metric.storage_read_size.unwrap_or(0);
        }
    }
    write!(
        target,
        "{prefix}_total_reads={total_reads},{prefix}_cache_misses={cache_misses},{prefix}_cache_miss_bytes={cache_miss_bytes}",
    )?;
    write!(target, "{prefix}_slowest_read={},{prefix}_slowest_read_name={slowest_read_name},{prefix}_slowest_read_key_size={slowest_read_key_size},{prefix}_slowest_read_storage_read_size={slowest_read_storage_read_size}", slowest_read.as_micros())?;
    Ok(())
}

/// Metrics on recent state accesses
#[derive(Debug)]
pub struct StateMetrics {
    #[allow(missing_docs)]
    accesses: Vec<StateAccessMetric>,
}

impl Default for StateMetrics {
    fn default() -> Self {
        Self {
            accesses: Vec::with_capacity(Self::MAX_CAPACITY),
        }
    }
}

impl StateMetrics {
    #[cfg(feature = "native")]
    const MAX_CAPACITY: usize = 100;

    #[cfg(not(feature = "native"))]
    const MAX_CAPACITY: usize = 0;

    /// Pushes a new state access metric.
    pub fn push(&mut self, metric: StateAccessMetric) {
        if self.accesses.len() < Self::MAX_CAPACITY {
            self.accesses.push(metric);
        }
    }

    /// Takes the state access metrics.
    pub fn take(&mut self) -> Vec<StateAccessMetric> {
        std::mem::take(&mut self.accesses)
    }

    /// Returns the number of accesses. in the current state metrics.
    pub fn len(&self) -> usize {
        self.accesses.len()
    }

    /// Returns true if there are no state accesses since the last flush
    pub fn is_empty(&self) -> bool {
        self.accesses.is_empty()
    }
}

/// Metrics for `auth_and_process_tx`, and the tx hash
#[derive(Debug)]
pub struct AuthAndProcessMetrics {
    /// The transaction hash
    pub tx_hash: HexHash,
    /// The metrics.
    pub timings: AuthAndProcessTimings,
}

impl AuthAndProcessMetrics {
    /// Creates a new `AuthAndProcessMetrics` instance.
    pub fn new(tx_hash: HexHash, timings: AuthAndProcessTimings) -> Self {
        Self { tx_hash, timings }
    }
}

#[cfg(feature = "native")]
impl Metric for AuthAndProcessMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_auth_and_process_metrics"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        use std::io::Write;
        write!(buffer, "{}, tx_hash={},total_time_us={},auth_time_us={},resolve_context_time_us={},check_uniqueness_time_us={},mark_tx_attempted_time_us={},attempt_tx_time_us={},reserve_gas_time_us={},refund_remaining_gas_time_us={},reward_prover_time_us={}", self.measurement_name(), self.tx_hash, self.timings.auth.elapsed().as_micros(), self.timings.total_timer.elapsed().as_micros(), self.timings.resolve_context_timer.elapsed().as_micros(), self.timings.check_uniqueness_timer.elapsed().as_micros(), self.timings.mark_tx_attempted_timer.elapsed().as_micros(), self.timings.attempt_tx_timer.elapsed().as_micros(), self.timings.reserve_gas_timer.elapsed().as_micros(), self.timings.refund_remaining_gas_timer.elapsed().as_micros(), self.timings.reward_prover_timer.elapsed().as_micros())?;
        summarize(
            &self.timings.resolve_context_access_metrics,
            "resolve_context",
            buffer,
        )?;
        summarize(
            &self.timings.check_uniqueness_access_metrics,
            "check_uniqueness",
            buffer,
        )?;
        summarize(
            &self.timings.mark_tx_attempted_access_metrics,
            "mark_tx_attempted",
            buffer,
        )?;
        summarize(
            &self.timings.attempt_tx_access_metrics,
            "attempt_tx",
            buffer,
        )?;
        summarize(
            &self.timings.reserve_gas_access_metrics,
            "reserve_gas",
            buffer,
        )?;
        summarize(
            &self.timings.refund_remaining_gas_access_metrics,
            "refund_remaining_gas",
            buffer,
        )?;
        summarize(
            &self.timings.reward_prover_access_metrics,
            "reward_prover",
            buffer,
        )?;
        Ok(())
    }
}

/// Timings for `auth_and_process_tx`
#[derive(Debug, Default)]
pub struct AuthAndProcessTimings {
    /// Time to deserialize and authenticate the tx. Includes no state access in the starter
    pub auth: MaybeTimer,
    /// The total time it took to authenticate and process the tx.
    pub total_timer: MaybeTimer,
    /// Timer for resolving the context.
    pub resolve_context_timer: MaybeTimer,
    /// State Accesses performed while resolving the context.
    pub resolve_context_access_metrics: Vec<StateAccessMetric>,
    /// Timer for checking uniqueness.
    pub check_uniqueness_timer: MaybeTimer,
    /// State Accesses performed while checking uniqueness.
    pub check_uniqueness_access_metrics: Vec<StateAccessMetric>,
    /// Timer for marking the tx as attempted.
    pub mark_tx_attempted_timer: MaybeTimer,
    /// State Accesses performed while marking the tx as attempted.
    pub mark_tx_attempted_access_metrics: Vec<StateAccessMetric>,
    /// Timer for executing the tx.
    pub attempt_tx_timer: MaybeTimer,
    /// State Accesses performed while executing the tx.
    pub attempt_tx_access_metrics: Vec<StateAccessMetric>,
    /// Timer for reserving gas.
    pub reserve_gas_timer: MaybeTimer,
    /// State Accesses performed while reserving gas.
    pub reserve_gas_access_metrics: Vec<StateAccessMetric>,
    /// Timer for refunding remaining gas.
    pub refund_remaining_gas_timer: MaybeTimer,
    /// State Accesses performed while refunding remaining gas.
    pub refund_remaining_gas_access_metrics: Vec<StateAccessMetric>,
    /// Timer for rewarding the prover.
    pub reward_prover_timer: MaybeTimer,
    /// State Accesses performed while rewarding the prover.
    pub reward_prover_access_metrics: Vec<StateAccessMetric>,
}
