//! InfluxDB metrics for Sovereign rollups.

use std::collections::HashMap;
use std::sync::LazyLock;
mod config;
#[cfg(feature = "gas-constant-estimation")]
mod csv_helper;
#[cfg(feature = "gas-constant-estimation")]
mod gas_constant_estimation;
mod publisher;
mod tracker;

pub use config::{MonitoringConfig, TelegrafSocketConfig};
#[cfg(feature = "gas-constant-estimation")]
pub use gas_constant_estimation::{GasConstantTracker, GAS_CONSTANTS};
pub use tracker::{
    init_metrics_tracker, timestamp, BatchMetrics, BatchOutcome, HttpMetrics, RunnerMetrics,
    RunnerProcessStfChangesMetrics, SlotProcessingMetrics, TransactionEffect,
    TransactionProcessingMetrics, UserSpaceSlotProcessingMetrics, ZkCircuit, ZkProvingTime,
    ZkVmExecutionChunk,
};

pub(crate) type SerializableMetric = Box<dyn Metric>;

/// Struct for tracking Sovereign metrics.
///
/// Hides underlying monitoring system implementation.
#[derive(Debug, Clone)]
pub struct MetricsTracker {
    sender: tokio::sync::mpsc::Sender<SerializableMetric>,
}

/// Anything that makes sense to serialize for telegraf.
pub trait Metric: Send + Sync + std::fmt::Debug {
    /// The name of the measurement for use in Flux queries.
    fn measurement_name(&self) -> &'static str;

    /// Write InfluxDb [`line protocol`](https://docs.influxdata.com/influxdb/cloud/reference/syntax/line-protocol/) format.
    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()>;

    /// Optionally writes metrics to a CSV file.  
    /// By default, this implementation does not write to the file.
    #[cfg(feature = "gas-constant-estimation")]
    fn write_to_csv(&self, _writers: &mut csv_helper::CsvWriters) -> std::io::Result<()> {
        Ok(())
    }
}

/// Applies a function to the global [`MetricsTracker`] instance.
pub fn track_metrics<F>(f: F)
where
    F: FnOnce(&MetricsTracker),
{
    match std::sync::OnceLock::get(&tracker::METRICS_TRACKER) {
        None => {
            tracing::warn!("Submitting metrics to uninitialized metrics tracker. Submitted metrics will be dropped. Please call `sov_metrics::init_metrics_tracker` to prevent data loss.");
        }
        Some(m) => {
            f(m);
        }
    };
}

/// A simple helper that replace characters from the input using the char map.
fn replace_chars(input: &str, char_map: &HashMap<char, &str>) -> String {
    let mut result = String::with_capacity(input.len());

    for c in input.chars() {
        match char_map.get(&c) {
            Some(replacement) => result.push_str(replacement),
            None => result.push(c),
        }
    }
    result
}

static TELEGRAF_ESCAPED_CHARS: LazyLock<HashMap<char, &'static str>> =
    LazyLock::new(|| HashMap::from([(' ', r"\ "), ('=', r"\="), (',', r"\,")]));

/// Returns a string that is the right format for telegraf.
/// Source: (Special telegraf characters)[`https://docs.influxdata.com/influxdb/cloud/reference/syntax/line-protocol/#special-characters`]
pub fn safe_telegraf_string(string: &str) -> String {
    replace_chars(string, &TELEGRAF_ESCAPED_CHARS)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::str::FromStr;

    use super::*;
    use crate::influxdb::config::TelegrafSocketConfig;
    use crate::influxdb::publisher::{
        metrics_publisher_task, receive_with_timeout, spawn_metrics_udp_receiver,
    };
    use crate::influxdb::tracker::timestamp;

    /// Starts publisher tasks and checks that tracker pushes all required metrics
    #[tokio::test(flavor = "multi_thread")]
    async fn test_runner_metrics_published() -> anyhow::Result<()> {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
        let monitoring_config = MonitoringConfig {
            telegraf_address: TelegrafSocketConfig::udp(socket.local_addr()?),
            // Setting low, so each metric is published immediately
            max_datagram_size: Some(1),
            max_pending_metrics: None,
        };

        let (metrics_back_sender, mut metrics_back_receiver) = tokio::sync::mpsc::channel(100);
        spawn_metrics_udp_receiver(socket, metrics_back_sender.clone());

        let (sender, receiver) = tokio::sync::mpsc::channel(10);
        let _task_handle = tokio::spawn(async move {
            metrics_publisher_task(receiver, &monitoring_config).await;
        });

        let tracker = MetricsTracker { sender };

        let start = timestamp();
        tracker.track_runner_metrics(RunnerMetrics {
            da_height: 12333,
            sync_distance: 55768,
            get_block_time: std::time::Duration::from_millis(1000),
            batches_processed: 2084,
            batch_bytes_processed: 785,
            transactions_processed: 4444,
            proofs_processed: 123854,
            proof_bytes_processed: 5432341,
            process_slot_time: std::time::Duration::from_millis(1001),
            apply_slot_time: std::time::Duration::from_millis(1002),
            stf_transition_time: std::time::Duration::from_millis(1003),
            extract_blobs_time: std::time::Duration::from_millis(1004),
            extraction_proof_time: std::time::Duration::from_millis(1005),
            processing_changes_time: std::time::Duration::from_millis(1006),
        });
        let finish = timestamp();

        // TODO: Verify exact metrics values;
        let total_expected_number_of_metrics = 14;
        let mut received_number_of_metrics = 0;
        loop {
            let metric = match receive_with_timeout(&mut metrics_back_receiver).await {
                None => break,
                Some(m) => m,
            };
            received_number_of_metrics += 1;
            let timestamp = u128::from_str(
                metric
                    .split(' ')
                    .next_back()
                    .expect("Timestamp not found for metric"),
            )
            .expect("Failed to parse timestamp");
            assert!(
                timestamp >= start,
                "Incorrect timestamp from metric: lagging behind"
            );
            assert!(
                timestamp <= finish,
                "Incorrect timestamp from metric: running upfront"
            );
        }

        assert_eq!(3, received_number_of_metrics);
        assert!(received_number_of_metrics <= total_expected_number_of_metrics);

        Ok(())
    }

    #[derive(Debug)]
    struct MyCustomMetric {
        value: u64,
        tag: u8,
    }

    impl Metric for MyCustomMetric {
        fn measurement_name(&self) -> &'static str {
            "my_custom_metric"
        }

        fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
            write!(
                buffer,
                "{} my_tag={} my_value={}",
                self.measurement_name(),
                self.tag,
                self.value,
            )
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_custom_metric_is_published() -> anyhow::Result<()> {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
        let monitoring_config = MonitoringConfig {
            telegraf_address: TelegrafSocketConfig::udp(socket.local_addr()?),
            // Setting low, so each metric is published immediately
            max_datagram_size: Some(1),
            max_pending_metrics: None,
        };

        let (metrics_back_sender, mut metrics_back_receiver) = tokio::sync::mpsc::channel(100);
        spawn_metrics_udp_receiver(socket, metrics_back_sender.clone());

        let (sender, receiver) = tokio::sync::mpsc::channel(10);
        let _task_handle = tokio::spawn(async move {
            metrics_publisher_task(receiver, &monitoring_config).await;
        });

        let tracker = MetricsTracker { sender };

        let my_metric = MyCustomMetric { value: 120, tag: 3 };

        tracker.submit(my_metric);

        let sent_metric = receive_with_timeout(&mut metrics_back_receiver)
            .await
            .unwrap();

        assert!(
            sent_metric.starts_with("my_custom_metric my_tag=3 my_value=120 "),
            "Metrics {sent_metric} does not contain expected prefix"
        );

        Ok(())
    }
}
