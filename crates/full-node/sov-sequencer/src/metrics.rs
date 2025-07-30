use std::io::Write;

use sov_metrics::Metric;

pub fn track_sequence_number(sequence_number: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit_inline(
            "sov_rollup_current_sequence_number",
            format!("current_sequence_number={sequence_number}"),
        );
    });
}

pub fn track_in_progress_batch_size(num_txs: u64) {
    sov_metrics::track_metrics(|tracker| {
        tracker.submit_inline(
            "sov_rollup_in_progress_batch_size",
            format!("num_txs={num_txs}"),
        );
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
