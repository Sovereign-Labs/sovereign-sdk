pub mod celestia;
#[cfg(feature = "native")]
pub mod checker;
#[cfg(feature = "native")]
mod config;
#[cfg(feature = "native")]
mod da_service;
#[cfg(feature = "native")]
mod metrics;
// Compressed-blob envelope (v1) parsing, decoding and encoding. The read/decode
// path is pure and ungated so the guest verifier shares identical logic; only the
// encoder is `native`-gated.
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
pub use config::{
    CelestiaConfig, CompressOnSubmit, GrpcEndpointConfig, RpcEndpointConfig, TxPriority,
    VerifyOnFetchMode,
};
#[cfg(feature = "native")]
pub use da_service::CelestiaService;

pub use crate::celestia::*;
