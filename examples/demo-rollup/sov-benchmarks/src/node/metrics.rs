//! In-process collection of the rollup's own metrics, for benches that attribute time to phases.

#![allow(clippy::float_arithmetic)]

use std::collections::HashMap;
use std::net::Ipv4Addr;

use sov_metrics::{init_metrics_tracker, MonitoringConfig, TelegrafSocketConfig};
use sov_shutdown::{BackgroundHandle, SecondaryShutdownController};
use tokio::io::AsyncBufReadExt;
use tokio::task::JoinHandle;

/// Pending-metrics queue. The default (1000) drops per-tx metrics at these rates.
const METRICS_QUEUE: u32 = 10_000_000;

/// Starts the process-global metrics tracker, publishing InfluxDB lines over TCP to an
/// in-process collector.
///
/// The queue is sized so no per-tx metrics are dropped; shutting `shutdown` down flushes them.
pub async fn start_metrics_collection(
    shutdown: &SecondaryShutdownController,
) -> anyhow::Result<(BackgroundHandle<()>, JoinHandle<Vec<String>>)> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let config = MonitoringConfig {
        telegraf_address: TelegrafSocketConfig::tcp(listener.local_addr()?),
        max_pending_metrics: Some(METRICS_QUEUE),
        ..MonitoringConfig::standard()
    };
    let publisher = init_metrics_tracker(&config, shutdown)
        .expect("The metrics tracker is initialized once, before the rollup starts");
    let collector = tokio::spawn(async move {
        let mut lines = Vec::new();
        // The publisher keeps one connection open until it shuts down. An accept or read error
        // only truncates the metrics; the summary reports how many lines arrived.
        if let Ok((stream, _)) = listener.accept().await {
            let mut reader = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                lines.push(line);
            }
        }
        lines
    });
    Ok((publisher, collector))
}

/// Field sums and line counts per (measurement, execution context), from the collected lines.
#[derive(Default)]
pub struct PhaseMetrics {
    /// Number of metric lines parsed.
    pub lines: usize,
    series: HashMap<(String, String), (HashMap<String, f64>, u64)>,
}

/// The publisher sometimes writes a metric right after the previous one's timestamp, without
/// a newline in between; split there (every measurement name starts with `sov_`).
fn split_glued(line: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = line
        .match_indices("sov_")
        .map(|(i, _)| i)
        .filter(|&i| i > 0 && line.as_bytes()[i - 1].is_ascii_digit())
        .collect();
    starts.insert(0, 0);
    starts.push(line.len());
    starts.windows(2).map(|w| &line[w[0]..w[1]]).collect()
}

impl PhaseMetrics {
    /// Parses InfluxDB lines as sent by the metrics publisher.
    pub fn from_lines(lines: &[String]) -> Self {
        let mut metrics = Self::default();
        for line in lines.iter().flat_map(|line| split_glued(line)) {
            metrics.lines += 1;
            // `measurement,tag=v,... field=v,... timestamp`
            let mut parts = line.splitn(3, ' ');
            let (Some(head), Some(fields)) = (parts.next(), parts.next()) else {
                continue;
            };
            let mut head = head.split(',');
            let measurement = head.next().unwrap_or_default().to_string();
            let context = head
                .find_map(|tag| tag.strip_prefix("context="))
                .unwrap_or_default()
                .to_string();
            let (sums, count) = metrics.series.entry((measurement, context)).or_default();
            *count += 1;
            for (field, value) in fields.split(',').filter_map(|kv| kv.split_once('=')) {
                // Skips string fields, which none of the summarized metrics use.
                if let Ok(value) = value.trim_end_matches('i').parse::<f64>() {
                    *sums.entry(field.to_string()).or_default() += value;
                }
            }
        }
        metrics
    }

    /// Sum of `field` over all lines of `measurement` in `context`; NaN if absent.
    pub fn sum(&self, measurement: &str, context: &str, field: &str) -> f64 {
        self.series
            .get(&(measurement.to_string(), context.to_string()))
            .and_then(|(sums, _)| sums.get(field).copied())
            .unwrap_or(f64::NAN)
    }

    /// Number of lines of `measurement` in `context`.
    pub fn count(&self, measurement: &str, context: &str) -> u64 {
        self.series
            .get(&(measurement.to_string(), context.to_string()))
            .map_or(0, |(_, count)| *count)
    }

    /// Average of `field` over all lines of `measurement` in `context`.
    pub fn avg(&self, measurement: &str, context: &str, field: &str) -> f64 {
        self.sum(measurement, context, field) / self.count(measurement, context) as f64
    }
}
