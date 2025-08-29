use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use sov_rollup_interface::common::HexHash;

use crate::MaybeTimer;
#[cfg(feature = "native")]
use crate::Metric;

type ArcFormatFn =
    Arc<dyn (Fn(&[u8], &mut fmt::Formatter<'_>) -> fmt::Result) + Send + Sync + 'static>;

/// Metrics for a single state access.
#[derive(Debug)]
pub struct StateAccessMetric {
    /// The key being accessed
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    key: MetricSlotKey,
    #[allow(missing_docs)]
    pub storage_read_size: Option<u32>,
    #[allow(missing_docs)]
    pub duration: MaybeTimer,
    /// The type of access.
    pub access_type: StateAccessType,
}

/// The type of state access.
#[derive(Debug)]
pub enum StateAccessType {
    /// Fetch the size of the value
    GetSize,
    /// Fetch the value itself
    GetValue,
}

/// A key for a metric.
pub struct MetricSlotKey {
    key: Arc<Vec<u8>>,
    display_fn: Option<ArcFormatFn>,
}

impl std::fmt::Display for MetricSlotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(display_fn) = &self.display_fn {
            display_fn(self.key.as_slice(), f)
        } else {
            write!(f, "unknown")
        }
    }
}
impl std::fmt::Debug for MetricSlotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(display_fn) = &self.display_fn {
            write!(f, "MetricKey {{ key: ")?;
            display_fn(self.key.as_slice(), f)?;
            write!(f, " }}")
        } else {
            write!(f, "MetricKey {{ key: {:?} }}", self.key.as_slice())
        }
    }
}

impl StateAccessMetric {
    /// Creates a new state access metric.
    pub fn new_size(key: Arc<Vec<u8>>, display_fn: Option<ArcFormatFn>) -> Self {
        Self {
            key: MetricSlotKey { key, display_fn },
            storage_read_size: None,
            duration: MaybeTimer::started(),
            access_type: StateAccessType::GetSize,
        }
    }

    /// Creates a new state access metric.
    pub fn new_read(key: Arc<Vec<u8>>, display_fn: Option<ArcFormatFn>) -> Self {
        Self {
            key: MetricSlotKey { key, display_fn },
            storage_read_size: None,
            duration: MaybeTimer::started(),
            access_type: StateAccessType::GetValue,
        }
    }

    /// Returns a serializable placeholder metric.
    pub fn placeholder() -> Self {
        Self {
            key: MetricSlotKey {
                key: Arc::new(vec![]),
                display_fn: None,
            },
            storage_read_size: None,
            duration: MaybeTimer::Completed(Duration::from_secs(0)),
            access_type: StateAccessType::GetSize,
        }
    }
}
#[cfg(feature = "native")]
fn summarize(metrics: &StateMetrics, prefix: &str, target: &mut Vec<u8>) -> std::io::Result<()> {
    use std::io::Write;
    let total_reads = metrics.total_reads;
    let cache_misses = metrics.total_read_misses;
    let cache_miss_bytes = metrics.total_read_bytes;
    let total_read_timing = metrics.total_read_timing.as_micros();
    let slowest_read = metrics.slowest_access.duration.elapsed();
    let slowest_read_storage_read_size = metrics.slowest_access.storage_read_size.unwrap_or(0);
    let slowest_deserialization_bytes = metrics.slowest_deserialize.deserialized_bytes;
    let slowest_deserialization_duration = metrics.slowest_deserialize.duration.as_micros();
    let total_deserialize_bytes = metrics.total_deserialize_bytes;
    let total_deserialize_duration = metrics.total_deserialize_timing.as_micros();
    write!(
        target,
        ",{prefix}_total_reads={total_reads},{prefix}_total_read_duration_us={total_read_timing},{prefix}_cache_misses={cache_misses},{prefix}_cache_miss_bytes={cache_miss_bytes},{prefix}_total_deserialize_bytes={total_deserialize_bytes},{prefix}_total_deserialize_duration_us={total_deserialize_duration}",
    )?;
    write!(target, ",{prefix}_slowest_read={},{prefix}_slowest_read_storage_read_size={slowest_read_storage_read_size}", slowest_read.as_micros())?;
    if metrics.slowest_access.key.display_fn.is_some() {
        write!(
            target,
            ",{prefix}_slowest_read_key=\"{}\"",
            metrics.slowest_access.key
        )?;
    }
    write!(target, ",{prefix}_slowest_deserialization_bytes={slowest_deserialization_bytes},{prefix}_slowest_deserialization_duration_us={slowest_deserialization_duration}")?;
    if metrics.slowest_deserialize.key.display_fn.is_some() {
        write!(
            target,
            ",{prefix}_slowest_deserialization_key=\"{}\"",
            metrics.slowest_deserialize.key
        )?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct SlowDeserialization {
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    key: MetricSlotKey,
    duration: Duration,
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    deserialized_bytes: u32,
}

impl SlowDeserialization {
    pub fn new(key: MetricSlotKey, duration: Duration, deserialized_bytes: u32) -> Self {
        Self {
            key,
            duration,
            deserialized_bytes,
        }
    }

    pub fn placeholder() -> Self {
        Self {
            key: MetricSlotKey {
                key: Arc::new(vec![]),
                display_fn: None,
            },
            duration: Duration::from_secs(0),
            deserialized_bytes: 0,
        }
    }
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
    /// The total number of bytes deserialized, including cache hits
    pub total_deserialize_bytes: u64,
    /// The total time spent deserializing.
    pub total_deserialize_timing: Duration,
    /// The slowest deserialization.
    pub slowest_deserialize: SlowDeserialization,
}

impl Default for StateMetrics {
    fn default() -> Self {
        Self {
            slowest_access: StateAccessMetric::placeholder(),
            total_reads: 0,
            total_read_misses: 0,
            total_read_timing: Duration::from_secs(0),
            total_read_bytes: 0,
            total_deserialize_bytes: 0,
            total_deserialize_timing: Duration::from_secs(0),
            slowest_deserialize: SlowDeserialization::placeholder(),
        }
    }
}

impl StateMetrics {
    /// Pushes a new state access metric.
    pub fn push(&mut self, mut metric: StateAccessMetric) {
        self.total_reads = self.total_reads.saturating_add(1);
        if let Some(size) = metric.storage_read_size {
            self.total_read_bytes = self.total_read_bytes.saturating_add(size as u64);
            self.total_read_misses = self.total_read_misses.saturating_add(1);
        }
        self.total_read_timing += metric.duration.stop_and_get_elapsed();
        if metric.duration.elapsed() > self.slowest_access.duration.elapsed() {
            self.slowest_access = metric;
        }
    }

    /// Adds metrics for a deserialization.
    pub fn add_deserialize_metric(
        &mut self,
        key_bytes: Arc<Vec<u8>>,
        format_fn: Option<ArcFormatFn>,
        deserialized_bytes: u32,
        duration: Duration,
    ) {
        let key = MetricSlotKey {
            key: key_bytes,
            display_fn: format_fn,
        };

        self.total_deserialize_bytes = self
            .total_deserialize_bytes
            .saturating_add(deserialized_bytes as u64);
        self.total_deserialize_timing += duration;
        if duration > self.slowest_deserialize.duration {
            self.slowest_deserialize = SlowDeserialization::new(key, duration, deserialized_bytes);
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
        let metric_name = self.measurement_name();
        let total_time_us = self.timings.total_timer.elapsed().as_micros();
        let auth_time_us = self.timings.auth.elapsed().as_micros();
        let resolve_context_time_us = self.timings.resolve_context_timer.elapsed().as_micros();
        let check_uniqueness_time_us = self.timings.check_uniqueness_timer.elapsed().as_micros();
        let mark_tx_attempted_time_us = self.timings.mark_tx_attempted_timer.elapsed().as_micros();
        let attempt_tx_time_us = self.timings.attempt_tx_timer.elapsed().as_micros();
        let reserve_gas_time_us = self.timings.reserve_gas_timer.elapsed().as_micros();
        let refund_remaining_gas_time_us = self
            .timings
            .refund_remaining_gas_timer
            .elapsed()
            .as_micros();
        let reward_prover_time_us = self.timings.reward_prover_timer.elapsed().as_micros();

        tracing::info!("AuthAndProcessMetrics: total_time_us={}", total_time_us);
        write!(buffer, "{metric_name} total_time_us={total_time_us},auth_time_us={auth_time_us},resolve_context_time_us={resolve_context_time_us},check_uniqueness_time_us={check_uniqueness_time_us},mark_tx_attempted_time_us={mark_tx_attempted_time_us},attempt_tx_time_us={attempt_tx_time_us},reserve_gas_time_us={reserve_gas_time_us},refund_remaining_gas_time_us={refund_remaining_gas_time_us},reward_prover_time_us={reward_prover_time_us}")?;
        summarize(
            &self.timings.attempt_tx_access_metrics,
            "attempt_tx",
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
