use std::net::SocketAddr;

use clap::{command, Parser, Subcommand};
use sov_metrics::TelegrafSocketConfig;

use super::{
    DEFAULT_BENCH_FILES, DEFAULT_INFLUX_DB_ADDRESS, DEFAULT_METRICS_OUTPUT, DEFAULT_NUM_THREADS,
    DEFAULT_TELEGRAF_ADDRESS,
};

#[derive(clap::Subcommand, Debug, Clone)]
pub enum MetricsQueryParameters {
    /// Only keep the following measurements.
    Measurements { sov_rollup_metrics: Vec<String> },
    /// Runs a custom query. Must be a valid flux query parameter.
    /// Examples of query parameters:
    /// ```ignore
    /// range(start: -1h)
    /// filter(fn: (r) => r._measurement == "example-measurement" and r._field == "example-field")
    /// filter(fn: (r) => r._measurement == "example-measurement_b" and r._field == "example-field_b")
    /// ```
    Custom { query_filters: Vec<String> },
}

impl MetricsQueryParameters {
    /// Formats the query filters into a valid flux query.
    pub(crate) fn format(self) -> String {
        let query_vec = match self {
            Self::Measurements { sov_rollup_metrics } => sov_rollup_metrics
                .iter()
                .map(|m| format!("r._measurement == \"{m}\""))
                .collect::<Vec<_>>(),
            Self::Custom { query_filters } => query_filters,
        };

        format!("|> filter(fn: (r) => {})", query_vec.join(" or "))
    }
}

#[derive(Parser, Clone, Debug)]
pub struct BenchRunnerCLI {
    /// Path to the bench files. It can be either a folder name or a specific file.
    /// If the path points to a folder, then all the bench files inside the folder will be run
    /// in a separate process.
    #[clap(short, long, default_value_t = DEFAULT_BENCH_FILES.to_string())]
    pub path: String,
    /// If set, then asserts the logs against the state. The inner value is the maximal number of
    /// concurrent requests to the node. If not specified, then no state assertions are performed.
    #[arg(short, long)]
    pub(crate) logs: Option<u8>,
    #[arg(short, long, default_value_t = DEFAULT_NUM_THREADS)]
    /// Maximum number of concurrent threads to run the benchmarks.
    pub(crate) threads: u8,
    /// Specifies how to store and query the metrics. If not specified, no metrics are stored.
    #[command(subcommand)]
    pub(crate) metrics: Option<MetricsCLI>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MetricsCLI {
    /// Track metrics using telegraf.
    Metrics {
        /// Address of the telegraf service. Make sure that the service is up and running before running this executable.
        #[arg(short, long, default_value_t = DEFAULT_TELEGRAF_ADDRESS)]
        telegraf: TelegrafSocketConfig,
        /// Address of the influxdb service. Make sure that the service is up and running before running this executable.
        #[arg(short, long, default_value_t = DEFAULT_INFLUX_DB_ADDRESS)]
        influx: SocketAddr,
        /// Influx token to authenticate with the influxdb service.
        #[arg(long)]
        influx_auth_token: String,
        /// Influx org id to authenticate with the influxdb service.
        #[arg(long)]
        influx_org_id: String,
        /// Output directory for the metrics.
        #[arg(short, long, default_value_t = DEFAULT_METRICS_OUTPUT.to_string())]
        output: String,
        /// Whether to encode the metrics in gzip format. By default, the metrics are not encoded.
        #[arg(short, long, default_value_t = false)]
        encoded: bool,
        /// Additional parameters to apply to the metrics. If not specified, then no filtering is applied.
        #[command(subcommand)]
        parameters: Option<MetricsQueryParameters>,
    },
}
