use std::io::Write;

use sov_metrics::Metric;

/// Gauge of the latest sequence number produced by the sequencer.
/// Emitted as `sov_rollup_current_sequence_number`. Should increase monotonically;
/// flatlining while the rollup is live indicates the sequencer is stalled.
#[derive(Debug)]
struct CurrentSequenceNumberMetric {
    /// Latest sequence number emitted.
    sequence_number: u64,
}

impl Metric for CurrentSequenceNumberMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_current_sequence_number"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} current_sequence_number={}",
            self.measurement_name(),
            self.sequence_number,
        )
    }
}

/// Gauge of the transaction count in the currently-open batch.
/// Emitted as `sov_rollup_in_progress_batch_size`. Growing unboundedly means the sequencer
/// is accumulating txs but not closing the batch; cross-check `sov_rollup_preferred_sequencer_channel`.
#[derive(Debug)]
struct InProgressBatchSizeMetric {
    /// Number of transactions in the currently-open batch.
    num_txs: u64,
}

impl Metric for InProgressBatchSizeMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_in_progress_batch_size"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} num_txs={}",
            self.measurement_name(),
            self.num_txs,
        )
    }
}

pub fn track_sequence_number(sequence_number: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(CurrentSequenceNumberMetric { sequence_number });
    });
}

pub fn track_in_progress_batch_size(num_txs: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(InProgressBatchSizeMetric { num_txs });
    });
}

/// Gauge of the signed gap between the sequencer's next-unassigned sequence number and
/// what the node observes. Emitted as `sov_rollup_sequence_number_delta`.
///
/// Brief non-zero values during rebase are normal; sustained non-zero = desync between
/// sequencer and node.
#[derive(Debug)]
struct SequenceNumberDeltaMetric {
    /// `sequencer_next - node_next`. Signed because either side can be ahead transiently.
    delta: i64,
}

impl Metric for SequenceNumberDeltaMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_sequence_number_delta"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(buffer, "{} delta={}i", self.measurement_name(), self.delta,)
    }
}

pub fn track_sequence_number_delta(delta: i64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit(SequenceNumberDeltaMetric { delta });
    });
}

#[derive(Debug)]
pub struct PreferredSequencerUpdateStateMetrics {
    pub duration: std::time::Duration,
    pub total_message_processing_duration: std::time::Duration,
    pub batches_count: u64,
    pub transactions_count: u64,
    pub in_progress_batch: bool,
    pub time_spent_fetching_batches: std::time::Duration,
}

impl Metric for PreferredSequencerUpdateStateMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_update_state"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} duration_ms={},total_message_processing_duration_ms={},fetch_batches_duration_us={},batches_count={},transactions_count={},in_progress_batch={}",
            self.measurement_name(),
            self.duration.as_millis(),
            self.total_message_processing_duration.as_millis(),
            self.time_spent_fetching_batches.as_micros(),
            self.batches_count,
            self.transactions_count,
            self.in_progress_batch
        )
    }
}

#[derive(Debug)]
pub struct PreferredSequencerChannelMetrics {
    pub duration: std::time::Duration,
    pub reason: &'static str,
    pub channel_size: u32,
}

impl Metric for PreferredSequencerChannelMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_channel"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},reason={} duration_us={},channel_size={}",
            self.measurement_name(),
            self.reason,
            self.duration.as_micros(),
            self.channel_size,
        )
    }
}

#[derive(Debug)]
pub struct PreferredSequencerChannelMetricsBatch {
    pub metrics: Vec<PreferredSequencerChannelMetrics>,
}

impl Metric for PreferredSequencerChannelMetricsBatch {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_channel"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        if self.metrics.is_empty() {
            return Ok(());
        }
        for (i, metric) in self.metrics.iter().enumerate() {
            metric.serialize_for_telegraf(buffer)?;
            if i != (self.metrics.len() - 1) {
                buffer.push(b'\n');
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct PreferredSequencerExecutorEventMetrics {
    pub event_type: &'static str,
    pub duration: std::time::Duration,
    pub batch_size: usize,
}

impl Metric for PreferredSequencerExecutorEventMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_executor_event"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},event_type={} duration_us={},batch_size={}",
            self.measurement_name(),
            self.event_type,
            self.duration.as_micros(),
            self.batch_size,
        )
    }
}

#[derive(Debug)]
pub struct PreferredSequencerFetchBatchesToReplayMetrics {
    pub duration: std::time::Duration,
    pub num_batches: u64,
    pub num_transactions: usize,
}

impl Metric for PreferredSequencerFetchBatchesToReplayMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_fetch_batches_to_replay"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} duration_us={},num_batches={},num_transactions={}",
            self.measurement_name(),
            self.duration.as_micros(),
            self.num_batches,
            self.num_transactions,
        )
    }
}

#[derive(Debug)]
pub struct PreferredSequencerSlotNumberMetrics {
    pub true_slot_number: u64,
    pub latest_finalized_slot_number: u64,
    pub node_visible_slot_number: u64,
    pub seq_visible_slot_number: u64,
}

impl Metric for PreferredSequencerSlotNumberMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_slot_numbers"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} true_slot={},last_finalized_slot={},node_visible_slot={},seq_visible_slot={}",
            self.measurement_name(),
            self.true_slot_number,
            self.latest_finalized_slot_number,
            self.node_visible_slot_number,
            self.seq_visible_slot_number,
        )
    }
}

#[derive(Debug)]
pub struct PreferredSequencerPruneMetrics {
    pub duration_ms: u64,
}

impl Metric for PreferredSequencerPruneMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_prune"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} duration_ms={}",
            self.measurement_name(),
            self.duration_ms,
        )
    }
}

/// The kind of write a preferred sequencer makes to Postgres.
#[derive(Debug, Clone, Copy, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum PostgresWriteOperation {
    BeginBatch,
    /// Persisting accepted transactions: the write that tx acceptance waits for.
    AddTxs,
    EndBatch,
    AddProof,
    Prune,
    /// Node registration and leader-election refresh. A small write of constant size, sent
    /// every heartbeat interval regardless of load, so it serves as a baseline round trip.
    Heartbeat,
}

/// How a Postgres write ended.
#[derive(Debug, Clone, Copy, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum PostgresWriteOutcome {
    Ok,
    /// The query went through, but the node is not the leader so nothing was written.
    NotLeader,
    /// Every attempt failed.
    Error,
}

/// One write to Postgres by the preferred sequencer, as observed by the sequencer: connection
/// acquisition, network round trip and the server-side write and commit.
/// Emitted as `sov_rollup_sequencer_postgres_write`.
#[derive(Debug)]
pub struct PostgresWriteMetric {
    pub operation: PostgresWriteOperation,
    pub outcome: PostgresWriteOutcome,
    /// The whole call, including retries and the backoff sleeps between them.
    pub duration: std::time::Duration,
    /// Time spent getting a connection from the pool in the last attempt. High values mean a
    /// new connection had to be opened.
    pub acquire: std::time::Duration,
    /// Time spent running the statement (or the whole transaction) in the last attempt.
    pub query: std::time::Duration,
    /// Number of attempts made. Above 1, `duration` includes retries.
    pub attempts: u32,
    /// Number of transactions written for `add_txs`, 1 for other operations.
    pub rows: u64,
    /// Size of the serialized transactions or blob sent, 0 when there is none.
    pub payload_bytes: u64,
}

impl Metric for PostgresWriteMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_sequencer_postgres_write"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let operation: &'static str = self.operation.into();
        let outcome: &'static str = self.outcome.into();
        write!(
            buffer,
            "{},operation={},outcome={} duration_us={},acquire_us={},query_us={},attempts={},rows={},payload_bytes={}",
            self.measurement_name(),
            operation,
            outcome,
            self.duration.as_micros(),
            self.acquire.as_micros(),
            self.query.as_micros(),
            self.attempts,
            self.rows,
            self.payload_bytes,
        )
    }
}

#[derive(Debug, Default)]
pub struct PreferredSequencerExecutorEventSendingMetrics {
    pub blocked_for_us: u64,
    pub queue_depth: usize,
}

impl Metric for PreferredSequencerExecutorEventSendingMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_preferred_sequencer_executor_event_sending"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} blocked_for_us={},queue_depth={}",
            self.measurement_name(),
            self.blocked_for_us,
            self.queue_depth,
        )
    }
}

/// Metric for main queue blocked time (sender side).
/// Only submitted when the send was actually blocked (blocked_for_us > 0).
#[derive(Debug)]
pub struct NonceBufferMainQueueBlockedMetric {
    /// How long the send was blocked waiting for capacity (microseconds).
    pub blocked_for_us: u64,
}

impl Metric for NonceBufferMainQueueBlockedMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_nonce_buffer_main_queue_blocked"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} blocked_for_us={}",
            self.measurement_name(),
            self.blocked_for_us,
        )
    }
}

/// Metric for main queue depth, meant to be used batched.
#[derive(Debug)]
pub struct NonceBufferMainQueueDepthMetric {
    /// Current depth of the queue.
    pub queue_depth: usize,
}

impl Metric for NonceBufferMainQueueDepthMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_nonce_buffer_main_queue_depth"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} queue_depth={}",
            self.measurement_name(),
            self.queue_depth,
        )
    }
}

/// Combined metric for timeout queue, meant to be used batched.
#[derive(Debug)]
pub struct NonceBufferTimeoutQueueMetric {
    /// How long the send was blocked waiting for capacity (microseconds).
    pub blocked_for_us: u64,
    /// Current depth of the queue after sending.
    pub queue_depth: usize,
}

impl Metric for NonceBufferTimeoutQueueMetric {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_nonce_buffer_timeout_queue"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} blocked_for_us={},queue_depth={}",
            self.measurement_name(),
            self.blocked_for_us,
            self.queue_depth,
        )
    }
}

/// Generic batch wrapper for any metric type.
/// Serializes multiple metrics separated by newlines.
#[derive(Debug)]
pub struct MetricBatch<M: Metric> {
    pub metrics: Vec<M>,
}

impl<M: Metric> Metric for MetricBatch<M> {
    fn measurement_name(&self) -> &'static str {
        // Return empty string since each metric has its own name
        // TODO: in practice measurement names are usually static, if this is always the case then
        // we could refactor it to not take `&self` and use `M::measurement_name()` here. #2264
        ""
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        if self.metrics.is_empty() {
            return Ok(());
        }
        for (i, metric) in self.metrics.iter().enumerate() {
            metric.serialize_for_telegraf(buffer)?;
            if i != (self.metrics.len() - 1) {
                buffer.push(b'\n');
            }
        }
        Ok(())
    }
}

/// Helper struct for batching metrics and flushing when batch is full.
/// Accumulates metrics and submits them in batches for efficiency.
pub struct MetricBatcher<M: Metric> {
    batch: Vec<M>,
    batch_size: usize,
}

impl<M: Metric + 'static> MetricBatcher<M> {
    /// Create a new batcher with the specified batch size.
    pub fn new(batch_size: usize) -> Self {
        Self {
            batch: Vec::with_capacity(batch_size),
            batch_size,
        }
    }

    /// Add a metric to the batch. Flushes automatically when batch is full.
    pub fn push(&mut self, metric: M) {
        self.batch.push(metric);
        if self.batch.len() >= self.batch_size {
            self.flush();
        }
    }

    /// Flush any accumulated metrics immediately.
    pub fn flush(&mut self) {
        if !self.batch.is_empty() {
            let metrics = std::mem::replace(&mut self.batch, Vec::with_capacity(self.batch_size));
            sov_metrics::track_metrics(|t| {
                t.submit(MetricBatch { metrics });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn postgres_write_metric_serializes_operation_and_outcome_as_tags() {
        let metric = PostgresWriteMetric {
            operation: PostgresWriteOperation::AddTxs,
            outcome: PostgresWriteOutcome::NotLeader,
            duration: Duration::from_micros(0x12345),
            acquire: Duration::from_micros(0x1234),
            query: Duration::from_micros(0x2345),
            attempts: 3,
            rows: 0x456,
            payload_bytes: 0x56789,
        };
        let mut buffer = Vec::new();
        metric.serialize_for_telegraf(&mut buffer).unwrap();

        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "sov_rollup_sequencer_postgres_write,operation=add_txs,outcome=not_leader \
             duration_us=74565,acquire_us=4660,query_us=9029,attempts=3,rows=1110,payload_bytes=354185",
        );
    }
}
