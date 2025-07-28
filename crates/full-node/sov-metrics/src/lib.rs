#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

/// Contains utilities to track zkVM cycles.
pub mod cycle_utils;

#[cfg(feature = "native")]
mod influxdb;

#[cfg(feature = "native")]
pub use influxdb::{
    init_metrics_tracker, safe_telegraf_string, timestamp, track_metrics, BatchMetrics,
    BatchOutcome, HttpMetrics, Metric, MetricsTracker, MonitoringConfig, RunnerMetrics,
    RunnerProcessStfChangesMetrics, SlotProcessingMetrics, TelegrafSocketConfig, TransactionEffect,
    TransactionProcessingMetrics, UserSpaceSlotProcessingMetrics, ZkCircuit, ZkProvingTime,
    ZkVmExecutionChunk,
};
#[cfg(all(feature = "native", feature = "gas-constant-estimation"))]
pub use influxdb::{GasConstantTracker, GAS_CONSTANTS};

/// Starts a timer if the `native` feature is enabled. Otherwise, does nothing.
#[macro_export]
macro_rules! start_timer {
    ($timer:ident) => {
        #[cfg(feature = "native")]
        let $timer = std::time::Instant::now();
    };
}

/// Returns the elapsed time since the timer if the `native` feature is enabled. Otherwise does nothing.
#[macro_export]
macro_rules! save_elapsed {
    ($end:ident SINCE $start:ident) => {
        #[cfg(feature = "native")]
        let $end = $start.elapsed();
    };
}
