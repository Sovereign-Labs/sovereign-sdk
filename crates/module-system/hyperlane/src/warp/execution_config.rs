use std::sync::OnceLock;

use sov_modules_api::{ExecutionInit, Spec};

use super::{Warp, WarpRouteId};

/// Global storage for the Warp execution configuration.
pub static WARP_EXECUTION_CONFIG: OnceLock<WarpExecutionConfig> = OnceLock::new();

/// Configuration for Warp module execution, specifically for metrics collection.
///
/// This configuration specifies which warp routes should have their rate limiter
/// metrics emitted during block processing.
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WarpExecutionConfig {
    /// Warp routes to monitor for rate limiter metrics.
    ///
    /// Metrics will be emitted for the rate limiters of these routes at the end
    /// of each block, labelled with the operator-supplied name.
    #[serde(default)]
    pub monitored_routes: Vec<MonitoredRoute>,
}

/// A warp route to emit rate limiter metrics for, paired with a human-readable name.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct MonitoredRoute {
    /// The id of the route to monitor.
    pub id: WarpRouteId,
    /// Human-readable name used to label the route's metrics.
    pub name: String,
}

impl<S: Spec> ExecutionInit for Warp<S> {
    type Config = WarpExecutionConfig;

    fn init(config: &Self::Config) -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!(?config, "Initializing Warp execution config");
        WARP_EXECUTION_CONFIG
            .set(config.clone())
            .map_err(|_| "Warp execution config already initialized")?;
        Ok(())
    }
}

impl<S: Spec> Warp<S> {
    /// Returns the warp routes that should be monitored for metrics.
    ///
    /// This reads from the global execution configuration set at startup.
    /// If no configuration is set, returns an empty slice.
    pub(super) fn get_monitored_routes(&self) -> &[MonitoredRoute] {
        WARP_EXECUTION_CONFIG
            .get()
            .map(|config| config.monitored_routes.as_slice())
            .unwrap_or_default()
    }
}
