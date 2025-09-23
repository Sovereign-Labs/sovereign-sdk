#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

/// Contains utilities to track zkVM cycles.
pub mod cycle_utils;

mod influx_db_nonnative;
#[cfg(feature = "native")]
mod influxdb;
mod maybe_timer;
pub use influx_db_nonnative::{
    AuthAndProcessMetrics, AuthAndProcessTimings, StateAccessMetric, StateMetrics,
};

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
pub use maybe_timer::MaybeTimer;
