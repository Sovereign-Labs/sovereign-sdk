#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

/// Contains utilities to track zkVM cycles.
pub mod cycle_utils;

mod influx_db_nonnative;
#[cfg(feature = "native")]
mod influxdb;
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

#[derive(Debug, Clone, Default)]
/// A metric, if the rollup is in native mode.
pub enum MaybeTimer {
    /// The event is in progress.
    InProgress(std::time::Instant),
    /// The event has completed.
    Completed(std::time::Duration),
    #[default]
    /// There is no event (because we're not in native mode).
    None,
}

#[cfg(not(feature = "native"))]
impl MaybeTimer {
    /// Starts a timer if the `native` feature is enabled. Otherwise, does nothing.
    pub fn start(&mut self) {}

    /// Starts a timer if the `native` feature is enabled. Otherwise, does nothing.
    pub fn started() -> Self {
        MaybeTimer::None
    }

    /// Ends the timer if the `native` feature is enabled. Otherwise, does nothing.
    /// Panics if the metric is not in progress.
    pub fn end(&mut self) {}

    /// Ends the timer if the `native` feature is enabled. Otherwise, does nothing.
    /// Panics if the metric is not in progress.
    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_secs(0)
    }

    /// Returns the elapsed time since the timer if the `native` feature is enabled. Otherwise does nothing.
    pub fn stop_and_get_elapsed(&mut self) -> std::time::Duration {
        std::time::Duration::from_secs(0)
    }
}

#[cfg(feature = "native")]
impl MaybeTimer {
    /// Starts a timer if the `native` feature is enabled. Otherwise, does nothing.
    pub fn start(&mut self) {
        *self = MaybeTimer::InProgress(std::time::Instant::now());
    }

    /// Starts a timer if the `native` feature is enabled. Otherwise, does nothing.
    pub fn started() -> Self {
        MaybeTimer::InProgress(std::time::Instant::now())
    }

    /// Ends the timer if the `native` feature is enabled. Otherwise, does nothing.
    /// Panics if the metric is not in progress.
    pub fn end(&mut self) {
        let MaybeTimer::InProgress(start) = self else {
            panic!("Cannot end a metric that is not in progress");
        };
        *self = MaybeTimer::Completed(start.elapsed());
    }

    /// Returns the elapsed time since the timer if the `native` feature is enabled. Otherwise does nothing.
    pub fn elapsed(&self) -> std::time::Duration {
        match self {
            MaybeTimer::InProgress(start) => start.elapsed(),
            MaybeTimer::Completed(duration) => *duration,
            MaybeTimer::None => std::time::Duration::from_secs(0),
        }
    }

    /// Returns the elapsed time since the timer if the `native` feature is enabled. Otherwise does nothing.
    pub fn stop_and_get_elapsed(&mut self) -> std::time::Duration {
        match self {
            MaybeTimer::InProgress(start) => {
                let duration = start.elapsed();
                *self = MaybeTimer::Completed(duration);
                duration
            }
            MaybeTimer::Completed(duration) => *duration,
            MaybeTimer::None => std::time::Duration::from_secs(0),
        }
    }
}
