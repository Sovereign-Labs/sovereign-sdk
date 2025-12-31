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
pub struct WarpExecutionConfig {
    /// List of warp route IDs to monitor for metrics.
    ///
    /// Metrics will be emitted for the rate limiters of these routes at the end
    /// of each block.
    #[serde(default)]
    pub monitored_route_ids: Vec<WarpRouteId>,
}

impl<S: Spec> ExecutionInit for Warp<S> {
    type Config = WarpExecutionConfig;

    fn init(config: &Self::Config) -> Result<(), Box<dyn std::error::Error>> {
        WARP_EXECUTION_CONFIG
            .set(config.clone())
            .map_err(|_| "Warp execution config already initialized")?;
        Ok(())
    }
}

impl<S: Spec> Warp<S> {
    /// Returns the list of warp route IDs that should be monitored for metrics.
    ///
    /// This reads from the global execution configuration set at startup.
    /// If no configuration is set, returns an empty vector.
    pub(super) fn get_monitored_route_ids(&self) -> &[WarpRouteId] {
        WARP_EXECUTION_CONFIG
            .get()
            .map(|config| config.monitored_route_ids.as_slice())
            .unwrap_or_default()
    }
}
