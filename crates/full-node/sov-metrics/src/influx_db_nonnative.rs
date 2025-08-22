use std::time::Duration;

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

    /// Returns a serializable placeholder metric.
    pub fn placeholder() -> Self {
        Self {
            op: "placeholder",
            key_size: 0,
            storage_read_size: None,
            duration: MaybeTimer::Completed(Duration::from_secs(0)),
        }
    }
}
#[cfg(feature = "native")]
fn summarize(metrics: &StateMetrics, prefix: &str, target: &mut Vec<u8>) -> std::io::Result<()> {
    use std::io::Write;
    let total_reads = metrics.total_reads;
    let cache_misses = metrics.total_read_misses;
    let cache_miss_bytes = metrics.total_read_bytes;
    let slowest_read = metrics.slowest_access.duration.elapsed();
    let slowest_read_name = metrics.slowest_access.op;
    let slowest_read_key_size = metrics.slowest_access.key_size;
    let slowest_read_storage_read_size = metrics.slowest_access.storage_read_size.unwrap_or(0);
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
    slowest_access: StateAccessMetric,
    /// The number of reads that were dropped because the metric was full.
    pub total_reads: u64,
    /// The number of reads that did not hit the cache.
    pub total_read_misses: u64,
    /// The total time spent reading from the state.
    pub total_read_timing: Duration,
    /// The total number of bytes read from the state.
    pub total_read_bytes: u64,
}

impl Default for StateMetrics {
    fn default() -> Self {
        Self {
            slowest_access: StateAccessMetric::placeholder(),
            total_reads: 0,
            total_read_misses: 0,
            total_read_timing: Duration::from_secs(0),
            total_read_bytes: 0,
        }
    }
}

impl StateMetrics {
    /// Pushes a new state access metric.
    pub fn push(&mut self, metric: StateAccessMetric) {
        self.total_reads = self.total_reads.saturating_add(1);
        if let Some(size) = metric.storage_read_size {
            self.total_read_bytes = self.total_read_bytes.saturating_add(size as u64);
            self.total_read_misses = self.total_read_misses.saturating_add(1);
        }
        self.total_read_timing += metric.duration.elapsed();
        if metric.duration.elapsed() > self.slowest_access.duration.elapsed() {
            self.slowest_access = metric;
        }
    }

    /// Takes the state access metrics.
    pub fn take(&mut self) -> StateMetrics {
        std::mem::take(self)
    }

    /// Returns the number of accesses. in the current state metrics.
    pub fn len(&self) -> usize {
        self.total_reads.try_into().expect("Performed more than 4 billion state accesses in a single block on a 32-bit system. This is impossible!")
    }

    /// Returns true if there are no state accesses since the last flush
    pub fn is_empty(&self) -> bool {
        self.total_reads == 0
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
    pub resolve_context_access_metrics: StateMetrics,
    /// Timer for checking uniqueness.
    pub check_uniqueness_timer: MaybeTimer,
    /// State Accesses performed while checking uniqueness.
    pub check_uniqueness_access_metrics: StateMetrics,
    /// Timer for marking the tx as attempted.
    pub mark_tx_attempted_timer: MaybeTimer,
    /// State Accesses performed while marking the tx as attempted.
    pub mark_tx_attempted_access_metrics: StateMetrics,
    /// Timer for executing the tx.
    pub attempt_tx_timer: MaybeTimer,
    /// State Accesses performed while executing the tx.
    pub attempt_tx_access_metrics: StateMetrics,
    /// Timer for reserving gas.
    pub reserve_gas_timer: MaybeTimer,
    /// State Accesses performed while reserving gas.
    pub reserve_gas_access_metrics: StateMetrics,
    /// Timer for refunding remaining gas.
    pub refund_remaining_gas_timer: MaybeTimer,
    /// State Accesses performed while refunding remaining gas.
    pub refund_remaining_gas_access_metrics: StateMetrics,
    /// Timer for rewarding the prover.
    pub reward_prover_timer: MaybeTimer,
    /// State Accesses performed while rewarding the prover.
    pub reward_prover_access_metrics: StateMetrics,
}
