pub mod celestia;
#[cfg(feature = "native")]
pub mod checker;
#[cfg(feature = "native")]
mod config;
#[cfg(feature = "native")]
mod da_service;
#[cfg(feature = "native")]
mod metrics;
// Compressed-blob envelope (v1) parsing and classification. Pure and ungated so
// the guest verifier shares identical logic. Wired into the read path in PR3.
#[allow(dead_code)]
mod envelope;
pub mod shares;
#[cfg(test)]
mod test_helper;
pub mod types;
pub mod verifier;

#[cfg(feature = "native")]
pub use sov_metrics::{init_metrics_tracker, MonitoringConfig};
pub use sov_rollup_interface::da::*;
#[cfg(feature = "native")]
pub use sov_rollup_interface::node::da::*;

#[cfg(feature = "native")]
pub use da_service::{CelestiaConfig, CelestiaService, VerifyOnFetchMode};

pub use crate::celestia::*;
