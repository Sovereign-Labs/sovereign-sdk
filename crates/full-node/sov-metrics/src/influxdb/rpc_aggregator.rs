//! In-process aggregation of per-call JSON-RPC statistics.
//!
//! Emitting one metric point per RPC request would overwhelm the metrics
//! pipeline: the publisher queue holds [`MonitoringConfig::max_pending_metrics`]
//! entries and silently drops on overflow, so a chatty indexer could crowd out
//! low-volume, high-value metrics (e.g. transaction submission). Instead, each
//! call is recorded as a small event on a bounded in-process channel; a
//! consumer task ([`RpcStatsAggregator::run_flush_loop`]) owns the statistics
//! map, accumulates events per `(method, transport)` pair, and flushes one
//! point per pair every [`RpcAggregationConfig::flush_interval_secs`].
//!
//! Recording is therefore bounded-time everywhere — a `try_send` plus a few
//! atomic operations, never a lock — which also makes it safe to record from
//! `Drop` implementations (used to observe cancelled calls). If the channel
//! overflows, events are dropped and counted; the consumer logs the lost count
//! each window.
//!
//! Unusually slow calls still emit individual points so they can be inspected
//! one by one, capped per window by
//! [`RpcAggregationConfig::max_slow_call_points_per_window`].
//!
//! [`MonitoringConfig::max_pending_metrics`]: crate::MonitoringConfig::max_pending_metrics

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::influxdb::tracker::timestamp;
use crate::influxdb::{safe_telegraf_string, Metric};
use crate::track_metrics;

/// Method-name tag used for calls whose method is not registered on the server.
/// Folding unregistered names into a single tag value prevents clients from
/// minting unbounded InfluxDB series by spamming bogus method names.
pub const UNKNOWN_METHOD: &str = "unknown";

/// Pseudo method-name tag under which whole-batch statistics are recorded.
///
/// The statistics map is keyed by string *content*, so a real registered
/// method literally named `batch` (or `unknown`) would merge with the
/// pseudo-method series. [`RpcStatsAggregator::new`] logs a warning when the
/// known-method set contains either reserved name.
pub const BATCH_PSEUDO_METHOD: &str = "batch";

/// Upper bounds (inclusive) of the latency histogram buckets, in milliseconds.
/// Calls slower than the last bound are only counted in the total call count,
/// which acts as the `+inf` bucket.
const BUCKET_BOUNDS_MS: [u64; 7] = [1, 5, 25, 100, 500, 2500, 10000];

/// Capacity of the recording channel. Sized to cover consumer scheduling
/// latency, not a full flush window: the consumer drains continuously, so the
/// channel only fills if the consumer is starved. Overflow drops events and
/// counts them in `lost_events`.
const EVENT_CHANNEL_CAPACITY: usize = 10_000;

/// Configuration for in-process aggregation of JSON-RPC call statistics.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct RpcAggregationConfig {
    /// How often aggregated RPC statistics are flushed to the metrics server,
    /// in seconds. One point per `(method, transport)` pair is emitted per
    /// window, and only for pairs that saw traffic.
    #[serde(default = "default_flush_interval_secs")]
    pub flush_interval_secs: u64,
    /// Calls at least this slow emit an individual metric point (subject to
    /// `max_slow_call_points_per_window`) and a debug-level log line with the
    /// request parameters.
    #[serde(default = "default_slow_call_threshold_ms")]
    pub slow_call_threshold_ms: u64,
    /// Maximum number of individual slow-call points emitted per flush window,
    /// so that a flood of slow queries degrades to "the aggregate shows many
    /// slow calls" instead of crowding out other metrics.
    #[serde(default = "default_max_slow_call_points_per_window")]
    pub max_slow_call_points_per_window: u32,
}

const fn default_flush_interval_secs() -> u64 {
    10
}

const fn default_slow_call_threshold_ms() -> u64 {
    500
}

const fn default_max_slow_call_points_per_window() -> u32 {
    10
}

impl RpcAggregationConfig {
    /// Default settings as a `const` constructor.
    pub const fn standard() -> Self {
        Self {
            flush_interval_secs: default_flush_interval_secs(),
            slow_call_threshold_ms: default_slow_call_threshold_ms(),
            max_slow_call_points_per_window: default_max_slow_call_points_per_window(),
        }
    }
}

impl Default for RpcAggregationConfig {
    fn default() -> Self {
        Self::standard()
    }
}

/// Statistics accumulated for a single `(method, transport)` pair within one
/// flush window.
#[derive(Debug, Default)]
struct MethodStats {
    /// Number of individually measured calls (single calls; for the `batch`
    /// pseudo-method, the number of batches).
    calls: u64,
    /// How many of `calls` returned a JSON-RPC error.
    errors: u64,
    /// Number of times this method appeared inside a batch. Batched entries
    /// are not individually timed (only the whole batch is), so they are
    /// counted separately to keep the duration statistics interpretable.
    batched_entries: u64,
    /// Entries inside completed batches whose response object contained an
    /// `error` member. Only ever non-zero on the batch pseudo-method: batch
    /// responses cannot be attributed to individual methods without parsing
    /// and correlating response IDs.
    failed_entries: u64,
    /// Calls whose in-flight future was dropped before completion (client
    /// disconnect or timeout). Not counted in `calls`, durations, or buckets,
    /// since they never produced a completion time.
    cancelled: u64,
    /// Client-to-server notifications. Notifications have no response, so
    /// they cannot error and are not timed; counting them separately keeps
    /// the `errors`/`calls` ratio meaningful.
    notifications: u64,
    total_duration_us: u128,
    max_duration_us: u128,
    /// Per-bucket (non-cumulative) call counts; see [`BUCKET_BOUNDS_MS`].
    bucket_counts: [u64; BUCKET_BOUNDS_MS.len()],
}

/// One flush-window's worth of statistics for a single `(method, transport)`
/// pair. Bucket fields are serialized cumulatively (Prometheus `le` style) so
/// percentiles can be estimated in Flux.
#[derive(Debug)]
pub struct AggregatedRpcMetrics {
    method: &'static str,
    is_ws: bool,
    stats: MethodStats,
}

impl Metric for AggregatedRpcMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_rpc_aggregated"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let method = safe_telegraf_string(self.method);
        write!(
            buffer,
            "{},method={method},is_ws={} calls={},errors={},batched_entries={},failed_entries={},cancelled={},notifications={},total_duration_us={},max_duration_us={}",
            self.measurement_name(),
            self.is_ws,
            self.stats.calls,
            self.stats.errors,
            self.stats.batched_entries,
            self.stats.failed_entries,
            self.stats.cancelled,
            self.stats.notifications,
            self.stats.total_duration_us,
            self.stats.max_duration_us,
        )?;
        let mut cumulative = 0u64;
        for (bound_ms, count) in BUCKET_BOUNDS_MS.iter().zip(self.stats.bucket_counts) {
            cumulative += count;
            write!(buffer, ",le_{bound_ms}ms={cumulative}")?;
        }
        Ok(())
    }
}

/// An individual RPC call that exceeded
/// [`RpcAggregationConfig::slow_call_threshold_ms`].
#[derive(Debug)]
pub struct SlowRpcCallMetrics {
    /// Method name, already resolved against the known-method set.
    pub method: &'static str,
    /// Whether the call arrived over a WebSocket connection.
    pub is_ws: bool,
    #[allow(missing_docs)]
    pub duration: Duration,
    /// JSON-RPC error code of the response; `None` if the call succeeded.
    /// Serialized as the `status` tag (`0` on success), matching the
    /// convention of `sov_rollup_rpc_handlers`.
    pub error_code: Option<i32>,
    /// Whether the call's future was dropped before completion; `duration` is
    /// then the elapsed time at the drop, not a completion time.
    pub cancelled: bool,
}

impl Metric for SlowRpcCallMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_rpc_slow_calls"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let method = safe_telegraf_string(self.method);
        write!(
            buffer,
            "{},method={method},is_ws={},status={},cancelled={} duration_us={}",
            self.measurement_name(),
            self.is_ws,
            self.error_code.unwrap_or(0),
            self.cancelled,
            self.duration.as_micros(),
        )
    }
}

/// Tells the caller how a recorded call was classified, so it can decide
/// whether to emit a slow-call log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedCall {
    /// The call was faster than the slow-call threshold.
    Fast,
    /// The call was at least as slow as the threshold. `point_emitted` is
    /// `false` once the per-window cap on individual slow-call points has been
    /// reached; the call is still counted in the aggregate either way.
    Slow {
        #[allow(missing_docs)]
        point_emitted: bool,
    },
}

impl RecordedCall {
    /// Whether the call was at least as slow as the slow-call threshold.
    pub fn is_slow(&self) -> bool {
        matches!(self, RecordedCall::Slow { .. })
    }
}

/// One recorded RPC observation, sent from the (hot) recording side to the
/// consumer task that owns the statistics map.
#[derive(Debug)]
enum RpcEvent {
    /// A completed, individually timed call.
    Call {
        method: &'static str,
        is_ws: bool,
        duration: Duration,
        error_code: Option<i32>,
    },
    /// A completed batch, timed as a whole under [`BATCH_PSEUDO_METHOD`].
    Batch {
        is_ws: bool,
        duration: Duration,
        error_code: Option<i32>,
        failed_entries: u64,
    },
    /// The (resolved) methods of one batch's entries, counted but untimed.
    BatchEntries {
        methods: Vec<&'static str>,
        is_ws: bool,
    },
    /// A client-to-server notification; untimed and cannot error.
    Notification { method: &'static str, is_ws: bool },
    /// A call whose future was dropped before completion.
    Cancelled { method: &'static str, is_ws: bool },
}

/// Records per-call RPC statistics as events on a bounded channel; the
/// consumer task ([`Self::run_flush_loop`]) aggregates them and periodically
/// flushes one point per `(method, transport)` pair to the global metrics
/// tracker. See the [module documentation](self) for the rationale.
///
/// Aggregated points are only emitted while [`Self::run_flush_loop`] is being
/// driven; without it, events accumulate until the channel fills and are then
/// dropped (counted, but never published). Individual slow-call points are
/// emitted directly from the recording path and so initially work without the
/// loop, but stop permanently once the per-window cap fills, because the cap
/// is only reset at flush time.
#[derive(Debug)]
pub struct RpcStatsAggregator {
    config: RpcAggregationConfig,
    known_methods: HashSet<&'static str>,
    events: tokio::sync::mpsc::Sender<RpcEvent>,
    /// Handed to [`Self::run_flush_loop`] on first invocation. This mutex is
    /// touched only at startup, never on the recording path.
    receiver: Mutex<Option<tokio::sync::mpsc::Receiver<RpcEvent>>>,
    /// Slow-call points emitted in the current window; reset by the consumer
    /// at each flush.
    slow_points_this_window: AtomicU32,
    /// Events dropped because the channel was full; reset and logged by the
    /// consumer at each flush.
    lost_events: AtomicU64,
}

impl RpcStatsAggregator {
    /// Creates an aggregator. `known_methods` should be the full set of
    /// methods registered on the server (e.g.
    /// [`jsonrpsee::server::Methods::method_names`]); calls to any other
    /// method are recorded under a single `unknown` tag to bound series
    /// cardinality.
    ///
    /// [`jsonrpsee::server::Methods::method_names`]: https://docs.rs/jsonrpsee/latest/jsonrpsee/server/struct.Methods.html#method.method_names
    pub fn new(
        config: RpcAggregationConfig,
        known_methods: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        let known_methods: HashSet<&'static str> = known_methods.into_iter().collect();
        for reserved in [BATCH_PSEUDO_METHOD, UNKNOWN_METHOD] {
            if known_methods.contains(reserved) {
                tracing::warn!(
                    method = reserved,
                    "Registered RPC method collides with a reserved metrics tag; \
                     its statistics will merge with the pseudo-method series"
                );
            }
        }
        let (events, receiver) = tokio::sync::mpsc::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            config,
            known_methods,
            events,
            receiver: Mutex::new(Some(receiver)),
            slow_points_this_window: AtomicU32::new(0),
            lost_events: AtomicU64::new(0),
        }
    }

    /// Resolves a wire method name against the known-method set, folding
    /// unregistered names into [`UNKNOWN_METHOD`]. The returned `&'static str`
    /// outlives the request, so callers can capture it before handing the
    /// request to the inner service.
    pub fn resolve_method(&self, method_name: &str) -> &'static str {
        self.known_methods
            .get(method_name)
            .copied()
            .unwrap_or(UNKNOWN_METHOD)
    }

    /// Records one completed call from its wire method name. Prefer
    /// [`Self::record_call_resolved`] when the name has already been resolved
    /// via [`Self::resolve_method`], to avoid a second lookup.
    pub fn record_call(
        &self,
        method_name: &str,
        is_ws: bool,
        duration: Duration,
        error_code: Option<i32>,
    ) -> RecordedCall {
        let method = self.resolve_method(method_name);
        self.record_call_resolved(method, is_ws, duration, error_code)
    }

    /// Records one completed call. `method` must come from
    /// [`Self::resolve_method`]; `error_code` is the JSON-RPC error code of
    /// the response, or `None` on success. If the call was slow, an individual
    /// [`SlowRpcCallMetrics`] point is submitted, subject to the per-window
    /// cap.
    pub fn record_call_resolved(
        &self,
        method: &'static str,
        is_ws: bool,
        duration: Duration,
        error_code: Option<i32>,
    ) -> RecordedCall {
        self.send_event(RpcEvent::Call {
            method,
            is_ws,
            duration,
            error_code,
        });
        self.maybe_emit_slow_point(method, is_ws, duration, error_code, false)
    }

    /// Records one completed batch under the `batch` pseudo-method. The
    /// duration covers the whole batch; per-entry methods should be reported
    /// via [`Self::record_batch_entries`].
    ///
    /// This method is required because jsonrpsee sometimes batches several calls - possibly to
    /// several separate underlying RPC methods - into a single invocation. When that happens,
    /// we receive results as a unit - so we can't report timing information separately for each individual method.
    ///
    /// `failed_entries` is the number of entries in the batch response that
    /// carried an error object — jsonrpsee marks the batch response itself as
    /// successful regardless of per-entry failures, so callers must count them
    /// from the response body. `error_code` only reflects whole-batch
    /// rejections (e.g. an oversized batch).
    pub fn record_batch(
        &self,
        is_ws: bool,
        duration: Duration,
        error_code: Option<i32>,
        failed_entries: u64,
    ) -> RecordedCall {
        self.send_event(RpcEvent::Batch {
            is_ws,
            duration,
            error_code,
            failed_entries,
        });
        self.maybe_emit_slow_point(BATCH_PSEUDO_METHOD, is_ws, duration, error_code, false)
    }

    /// Counts the entries of one batch. Batched entries are not individually
    /// timed, so this only increments counters. The names must come from
    /// [`Self::resolve_method`].
    pub fn record_batch_entries(&self, methods: Vec<&'static str>, is_ws: bool) {
        if !methods.is_empty() {
            self.send_event(RpcEvent::BatchEntries { methods, is_ws });
        }
    }

    /// Counts one client-to-server notification. Notifications have no
    /// response, so they cannot error and are not timed. `method` must come
    /// from [`Self::resolve_method`].
    pub fn record_notification(&self, method: &'static str, is_ws: bool) {
        self.send_event(RpcEvent::Notification { method, is_ws });
    }

    /// Records a call whose future was dropped before completion (client
    /// disconnect or timeout). `elapsed` is the time from the start of the
    /// call to the drop; if it already exceeds the slow-call threshold, an
    /// individual slow point is emitted (subject to the per-window cap) with
    /// `cancelled=true`.
    ///
    /// This only performs a `try_send` and atomic operations, making it safe
    /// to call from `Drop` implementations.
    pub fn record_cancelled(
        &self,
        method: &'static str,
        is_ws: bool,
        elapsed: Duration,
    ) -> RecordedCall {
        self.send_event(RpcEvent::Cancelled { method, is_ws });
        self.maybe_emit_slow_point(method, is_ws, elapsed, None, true)
    }

    /// [`Self::record_cancelled`] for a whole batch, recorded under the
    /// `batch` pseudo-method.
    pub fn record_cancelled_batch(&self, is_ws: bool, elapsed: Duration) -> RecordedCall {
        self.record_cancelled(BATCH_PSEUDO_METHOD, is_ws, elapsed)
    }

    /// Runs the consumer loop: applies recorded events to the statistics map,
    /// flushes aggregated points every
    /// [`RpcAggregationConfig::flush_interval_secs`], and performs a final
    /// flush (after draining pending events) when `shutdown` resolves.
    ///
    /// Must be driven for aggregated points to be emitted at all; callers are
    /// expected to `tokio::spawn` it and observe the task's completion.
    /// Subsequent invocations log a warning and return immediately.
    pub async fn run_flush_loop<F>(self: std::sync::Arc<Self>, shutdown: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(mut receiver) = receiver else {
            tracing::warn!("RPC stats flush loop invoked more than once; ignoring this invocation");
            return;
        };

        let mut stats = HashMap::new();
        // `max(1)` because `tokio::time::interval` panics on a zero period.
        let period = Duration::from_secs(self.config.flush_interval_secs.max(1));
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => {
                    // Drain events recorded before shutdown; `try_recv` errs
                    // once the channel is empty, which ends the drain.
                    while let Ok(event) = receiver.try_recv() {
                        apply(&mut stats, event);
                    }
                    self.flush(&mut stats);
                    return;
                }
                maybe_event = receiver.recv() => match maybe_event {
                    Some(event) => apply(&mut stats, event),
                    // All senders dropped: nothing can be recorded anymore.
                    None => {
                        self.flush(&mut stats);
                        return;
                    }
                },
                _ = interval.tick() => self.flush(&mut stats),
            }
        }
    }

    /// Submits one point per `(method, transport)` pair that saw traffic, all
    /// with the same timestamp, and resets the per-window state (slow-call
    /// cap, lost-event counter, statistics map).
    fn flush(&self, stats: &mut HashMap<(&'static str, bool), MethodStats>) {
        self.slow_points_this_window.store(0, Ordering::Relaxed);
        let lost = self.lost_events.swap(0, Ordering::Relaxed);
        if lost > 0 {
            tracing::warn!(
                lost_events = lost,
                "RPC metrics event channel overflowed; some calls were not aggregated"
            );
        }
        if stats.is_empty() {
            return;
        }
        let drained = std::mem::take(stats);
        let ts = timestamp();
        track_metrics(move |tracker| {
            for ((method, is_ws), stats) in drained {
                tracker.submit_with_time(
                    ts,
                    AggregatedRpcMetrics {
                        method,
                        is_ws,
                        stats,
                    },
                );
            }
        });
    }

    fn slow_threshold(&self) -> Duration {
        Duration::from_millis(self.config.slow_call_threshold_ms)
    }

    /// Classifies a duration against the slow-call threshold and, when slow,
    /// emits an individual [`SlowRpcCallMetrics`] point subject to the
    /// per-window cap.
    fn maybe_emit_slow_point(
        &self,
        method: &'static str,
        is_ws: bool,
        duration: Duration,
        error_code: Option<i32>,
        cancelled: bool,
    ) -> RecordedCall {
        if duration < self.slow_threshold() {
            return RecordedCall::Fast;
        }
        let point_emitted = self.slow_points_this_window.fetch_add(1, Ordering::Relaxed)
            < self.config.max_slow_call_points_per_window;
        if point_emitted {
            track_metrics(|tracker| {
                tracker.submit(SlowRpcCallMetrics {
                    method,
                    is_ws,
                    duration,
                    error_code,
                    cancelled,
                });
            });
        }
        RecordedCall::Slow { point_emitted }
    }

    fn send_event(&self, event: RpcEvent) {
        // Dropping on overflow keeps recording bounded-time; the consumer
        // logs the lost count each window, so the loss is visible.
        if self.events.try_send(event).is_err() {
            self.lost_events.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Applies one recorded event to the consumer's statistics map.
fn apply(stats: &mut HashMap<(&'static str, bool), MethodStats>, event: RpcEvent) {
    match event {
        RpcEvent::Call {
            method,
            is_ws,
            duration,
            error_code,
        } => {
            let entry = stats.entry((method, is_ws)).or_default();
            entry.calls += 1;
            if error_code.is_some() {
                entry.errors += 1;
            }
            record_duration(entry, duration);
        }
        RpcEvent::Batch {
            is_ws,
            duration,
            error_code,
            failed_entries,
        } => {
            let entry = stats.entry((BATCH_PSEUDO_METHOD, is_ws)).or_default();
            entry.calls += 1;
            if error_code.is_some() {
                entry.errors += 1;
            }
            entry.failed_entries += failed_entries;
            record_duration(entry, duration);
        }
        RpcEvent::BatchEntries { methods, is_ws } => {
            for method in methods {
                stats.entry((method, is_ws)).or_default().batched_entries += 1;
            }
        }
        RpcEvent::Notification { method, is_ws } => {
            stats.entry((method, is_ws)).or_default().notifications += 1;
        }
        RpcEvent::Cancelled { method, is_ws } => {
            stats.entry((method, is_ws)).or_default().cancelled += 1;
        }
    }
}

fn record_duration(entry: &mut MethodStats, duration: Duration) {
    let duration_us = duration.as_micros();
    entry.total_duration_us += duration_us;
    entry.max_duration_us = entry.max_duration_us.max(duration_us);
    // Compare full-precision durations: truncating to whole milliseconds
    // would misclassify e.g. a 1.9ms call into the `le_1ms` bucket.
    if let Some(bucket) = BUCKET_BOUNDS_MS
        .iter()
        .position(|bound| duration <= Duration::from_millis(*bound))
    {
        entry.bucket_counts[bucket] += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: [&str; 2] = ["eth_getLogs", "eth_sendRawTransaction"];

    fn aggregator() -> RpcStatsAggregator {
        RpcStatsAggregator::new(RpcAggregationConfig::standard(), KNOWN)
    }

    /// Receives all queued events and applies them, returning the resulting
    /// points — the synchronous equivalent of one flush window.
    fn drain(agg: &RpcStatsAggregator) -> Vec<AggregatedRpcMetrics> {
        let mut receiver = agg
            .receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let receiver = receiver
            .as_mut()
            .expect("tests do not run the flush loop, so the receiver is present");
        let mut stats = HashMap::new();
        // Drain until empty: `try_recv` errs once no events are queued.
        while let Ok(event) = receiver.try_recv() {
            apply(&mut stats, event);
        }
        stats
            .into_iter()
            .map(|((method, is_ws), stats)| AggregatedRpcMetrics {
                method,
                is_ws,
                stats,
            })
            .collect()
    }

    fn point_for<'a>(
        points: &'a [AggregatedRpcMetrics],
        method: &str,
        is_ws: bool,
    ) -> &'a AggregatedRpcMetrics {
        points
            .iter()
            .find(|p| p.method == method && p.is_ws == is_ws)
            .unwrap_or_else(|| panic!("no point for ({method}, is_ws={is_ws})"))
    }

    #[test]
    fn calls_aggregate_into_one_point_per_method_and_transport() {
        let agg = aggregator();
        agg.record_call("eth_getLogs", false, Duration::from_millis(20), None);
        agg.record_call(
            "eth_getLogs",
            false,
            Duration::from_millis(80),
            Some(-32000),
        );
        agg.record_call("eth_getLogs", true, Duration::from_millis(3), None);

        let points = drain(&agg);
        assert_eq!(
            points.len(),
            2,
            "expected one point per (method, transport)"
        );

        let http = point_for(&points, "eth_getLogs", false);
        assert_eq!(http.stats.calls, 2);
        assert_eq!(http.stats.errors, 1);
        assert_eq!(http.stats.total_duration_us, 100_000);
        assert_eq!(http.stats.max_duration_us, 80_000);

        let ws = point_for(&points, "eth_getLogs", true);
        assert_eq!(ws.stats.calls, 1);
    }

    #[test]
    fn unregistered_methods_fold_into_unknown_tag() {
        let agg = aggregator();
        agg.record_call("made_up_method_1", false, Duration::from_millis(1), None);
        agg.record_call("made_up_method_2", false, Duration::from_millis(1), None);

        let points = drain(&agg);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].method, UNKNOWN_METHOD);
        assert_eq!(points[0].stats.calls, 2);
    }

    #[test]
    fn batch_entries_count_separately_from_timed_calls() {
        let agg = aggregator();
        agg.record_batch_entries(vec!["eth_getLogs", "eth_getLogs"], false);
        agg.record_batch(false, Duration::from_millis(50), None, 0);

        let points = drain(&agg);
        let logs = point_for(&points, "eth_getLogs", false);
        assert_eq!(logs.stats.batched_entries, 2);
        assert_eq!(
            logs.stats.calls, 0,
            "batched entries must not inflate the individually-timed call count"
        );

        let batch = point_for(&points, BATCH_PSEUDO_METHOD, false);
        assert_eq!(batch.stats.calls, 1);
        assert_eq!(batch.stats.total_duration_us, 50_000);
    }

    #[test]
    fn resolve_method_folds_unregistered_names() {
        let agg = aggregator();
        assert_eq!(agg.resolve_method("eth_getLogs"), "eth_getLogs");
        assert_eq!(agg.resolve_method("bogus_method"), UNKNOWN_METHOD);
    }

    #[test]
    fn batch_failed_entries_are_recorded_on_batch_pseudo_method() {
        let agg = aggregator();
        agg.record_batch(false, Duration::from_millis(10), None, 2);

        let points = drain(&agg);
        let batch = point_for(&points, BATCH_PSEUDO_METHOD, false);
        assert_eq!(batch.stats.failed_entries, 2);
        assert_eq!(
            batch.stats.errors, 0,
            "per-entry failures must not count as whole-batch errors"
        );
    }

    #[test]
    fn notifications_count_separately_from_calls() {
        let agg = aggregator();
        agg.record_notification("eth_getLogs", false);

        let points = drain(&agg);
        let logs = point_for(&points, "eth_getLogs", false);
        assert_eq!(logs.stats.notifications, 1);
        assert_eq!(logs.stats.calls, 0);
    }

    #[test]
    fn cancelled_calls_do_not_count_as_calls() {
        let agg = aggregator();
        agg.record_cancelled("eth_getLogs", false, Duration::from_millis(10));

        let points = drain(&agg);
        let logs = point_for(&points, "eth_getLogs", false);
        assert_eq!(logs.stats.cancelled, 1);
        assert_eq!(logs.stats.calls, 0);
        assert_eq!(
            logs.stats.total_duration_us, 0,
            "cancelled calls have no completion time and must not skew durations"
        );
    }

    #[test]
    fn slow_cancelled_call_consumes_slow_point_budget() {
        let config = RpcAggregationConfig {
            slow_call_threshold_ms: 100,
            max_slow_call_points_per_window: 1,
            ..RpcAggregationConfig::standard()
        };
        let agg = RpcStatsAggregator::new(config, KNOWN);

        assert_eq!(
            agg.record_cancelled("eth_getLogs", false, Duration::from_millis(150)),
            RecordedCall::Slow {
                point_emitted: true
            }
        );
        assert_eq!(
            agg.record_call("eth_getLogs", false, Duration::from_millis(150), None),
            RecordedCall::Slow {
                point_emitted: false
            },
            "slow cancelled point should count against the per-window cap"
        );
    }

    #[test]
    fn flush_consumes_window_state() {
        let agg = aggregator();
        agg.record_call("eth_getLogs", false, Duration::from_millis(1), None);
        let mut stats = HashMap::new();
        {
            let mut receiver = agg.receiver.lock().unwrap();
            // Drain until empty: `try_recv` errs once no events are queued.
            while let Ok(event) = receiver.as_mut().unwrap().try_recv() {
                apply(&mut stats, event);
            }
        }

        agg.flush(&mut stats);
        assert!(
            stats.is_empty(),
            "flush should leave an empty map for the next window"
        );
    }

    #[test]
    fn slow_calls_are_flagged_and_capped_per_window() {
        let config = RpcAggregationConfig {
            slow_call_threshold_ms: 100,
            max_slow_call_points_per_window: 2,
            ..RpcAggregationConfig::standard()
        };
        let agg = RpcStatsAggregator::new(config, KNOWN);
        let slow = Duration::from_millis(150);

        assert_eq!(
            agg.record_call("eth_getLogs", false, Duration::from_millis(50), None),
            RecordedCall::Fast
        );
        assert_eq!(
            agg.record_call("eth_getLogs", false, slow, None),
            RecordedCall::Slow {
                point_emitted: true
            }
        );
        assert_eq!(
            agg.record_call("eth_getLogs", false, slow, None),
            RecordedCall::Slow {
                point_emitted: true
            }
        );
        assert_eq!(
            agg.record_call("eth_getLogs", false, slow, None),
            RecordedCall::Slow {
                point_emitted: false
            },
            "third slow call in the window should exceed the cap of 2"
        );

        agg.flush(&mut HashMap::new());
        assert_eq!(
            agg.record_call("eth_getLogs", false, slow, None),
            RecordedCall::Slow {
                point_emitted: true
            },
            "slow-call cap should reset when the window is flushed"
        );
    }

    #[test]
    fn bucket_classification_does_not_truncate_sub_millisecond_durations() {
        let agg = aggregator();
        agg.record_call("eth_getLogs", false, Duration::from_micros(1000), None);
        agg.record_call("eth_getLogs", false, Duration::from_micros(1500), None);

        let points = drain(&agg);
        let logs = point_for(&points, "eth_getLogs", false);
        assert_eq!(
            logs.stats.bucket_counts,
            [1, 1, 0, 0, 0, 0, 0],
            "a 1.5ms call must land in le_5ms, not le_1ms"
        );
    }

    #[test]
    fn boundary_duration_agrees_between_bucket_and_slow_flag() {
        let agg = aggregator();
        // 500.4ms with the default 500ms threshold: must be Slow AND must not
        // be counted in the le_500ms bucket.
        let recorded = agg.record_call("eth_getLogs", false, Duration::from_micros(500_400), None);
        assert_eq!(
            recorded,
            RecordedCall::Slow {
                point_emitted: true
            }
        );

        let points = drain(&agg);
        let logs = point_for(&points, "eth_getLogs", false);
        assert_eq!(
            logs.stats.bucket_counts,
            [0, 0, 0, 0, 0, 1, 0],
            "a 500.4ms call belongs in le_2500ms, consistent with being flagged slow"
        );
    }

    #[test]
    fn aggregated_point_serializes_cumulative_buckets() {
        let agg = aggregator();
        agg.record_call("eth_getLogs", false, Duration::from_millis(3), None);
        agg.record_call("eth_getLogs", false, Duration::from_millis(80), None);
        // Slower than the largest bucket bound: counted only in `calls`.
        agg.record_call("eth_getLogs", false, Duration::from_secs(20), None);

        let points = drain(&agg);
        let mut buffer = Vec::new();
        points[0].serialize_for_telegraf(&mut buffer).unwrap();
        let line = String::from_utf8(buffer).unwrap();

        assert_eq!(
            line,
            "sov_rollup_rpc_aggregated,method=eth_getLogs,is_ws=false \
             calls=3,errors=0,batched_entries=0,failed_entries=0,cancelled=0,notifications=0,\
             total_duration_us=20083000,max_duration_us=20000000,\
             le_1ms=0,le_5ms=1,le_25ms=1,le_100ms=2,le_500ms=2,le_2500ms=2,le_10000ms=2"
        );
    }

    #[test]
    fn slow_call_point_serializes_tags_and_fields() {
        let metric = SlowRpcCallMetrics {
            method: "eth_getLogs",
            is_ws: true,
            duration: Duration::from_millis(1234),
            error_code: Some(-32603),
            cancelled: false,
        };
        let mut buffer = Vec::new();
        metric.serialize_for_telegraf(&mut buffer).unwrap();
        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "sov_rollup_rpc_slow_calls,method=eth_getLogs,is_ws=true,status=-32603,cancelled=false duration_us=1234000"
        );
    }

    #[test]
    fn cancelled_slow_call_point_serializes_cancelled_tag() {
        let metric = SlowRpcCallMetrics {
            method: "eth_getLogs",
            is_ws: false,
            duration: Duration::from_millis(700),
            error_code: None,
            cancelled: true,
        };
        let mut buffer = Vec::new();
        metric.serialize_for_telegraf(&mut buffer).unwrap();
        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "sov_rollup_rpc_slow_calls,method=eth_getLogs,is_ws=false,status=0,cancelled=true duration_us=700000"
        );
    }
}
