//! Configuration for [`crate::da_service::CelestiaService`]
use std::num::NonZero;

use schemars::JsonSchema;

use crate::verifier::address::CelestiaAddress;

/// Runtime configuration for the [`sov_rollup_interface::node::da::DaService`] implementation.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CelestiaConfig {
    /// The JWT used to authenticate with the Celestia RPC server
    pub celestia_rpc_auth_token: String,
    /// The address of the Celestia RPC server
    #[serde(default = "default_rpc_addr")]
    pub celestia_rpc_address: String,
    /// The maximum size of a Celestia RPC response, in bytes
    #[serde(default = "default_max_response_size")]
    pub max_celestia_response_body_size: NonZero<u32>,
    /// The timeout for a Celestia RPC request, in seconds
    #[serde(default = "default_request_timeout_seconds")]
    pub celestia_rpc_timeout_seconds: NonZero<u64>,
    /// See [`sov_rollup_interface::node::da::DaService::safe_lead_time`].
    #[serde(default = "default_safe_lead_time_ms")]
    pub safe_lead_time_ms: u64,
    /// The sequencer address that will be used as the signer for the blobs.
    /// CelestiaService fetches the signer address from the Celestia RPC server.
    /// Set it only to ensure that the target node runs with correct credentials.
    pub signer_address: Option<CelestiaAddress>,

    /// Minimal time to wait before reattempting to request to celestia node.
    /// See [`backon::ExponentialBuilder`] for more details
    #[serde(default = "default_min_delay_ms")]
    pub backoff_min_delay_ms: u64,
    /// Maximal time between reattempting to request to the celestia node.
    /// See [`backon::ExponentialBuilder`] for more details
    #[serde(default = "default_max_delay_ms")]
    pub backoff_max_delay_ms: u64,
    /// Number of requests attempted on the celestia node before returning an error.
    /// See [`backon::ExponentialBuilder`] for more details
    #[serde(default = "default_max_times")]
    pub backoff_max_times: usize,
    /// Exponential factor for reattempting failed requests
    /// See [`backon::ExponentialBuilder`] for more details
    #[serde(default = "default_factor")]
    pub backoff_factor: f32,
}

impl CelestiaConfig {
    pub(crate) fn get_backoff_policy(&self) -> backon::ExponentialBuilder {
        let backoff_policy = backon::ExponentialBuilder::default()
            .with_min_delay(std::time::Duration::from_millis(self.backoff_min_delay_ms))
            .with_max_times(self.backoff_max_times)
            .with_max_delay(std::time::Duration::from_millis(self.backoff_max_delay_ms))
            .with_factor(self.backoff_factor);

        tracing::debug!(?backoff_policy, "Configured backoff policy");
        backoff_policy
    }

    #[cfg(test)]
    pub(crate) fn dev_config(url: &str) -> Self {
        Self {
            celestia_rpc_auth_token: "TEST".to_string(),
            celestia_rpc_address: url.to_string(),
            max_celestia_response_body_size: NonZero::new(1024 * 1024 * 100).unwrap(),
            celestia_rpc_timeout_seconds: NonZero::new(120).unwrap(),
            safe_lead_time_ms: 500,
            signer_address: None,
            backoff_min_delay_ms: 50,
            backoff_max_delay_ms: 100,
            backoff_max_times: 3,
            backoff_factor: default_factor(),
        }
    }
}

fn default_safe_lead_time_ms() -> u64 {
    500
}

fn default_rpc_addr() -> String {
    "http://localhost:11111/".into()
}

fn default_max_response_size() -> NonZero<u32> {
    // 100 MiB
    NonZero::new(1024 * 1024 * 100).unwrap()
}

// Exponential backoff defaults:
// **Timing for Each Attempt:**
// 1. Attempt 1: 100ms
// 2. Attempt 2: 200ms
// 3. Attempt 3: 400ms
// 4. Attempt 4: 800ms
// 5. Attempt 5: 1.6s
// 6. Attempt 6: 3.2s
// 7. Attempt 7: 6.4s
// 8. Attempt 8: 12.8s
// 9. Attempt 9: 25.6s
// 10. Attempt 10: 30s (capped at max_delay)
// 11. Attempt 11-60: 30s each
// **Total Number of Attempts:** 60 (as specified by ) `with_max_times(60)`
// **Total Waiting Time:**
// - First 9 attempts: 100ms + 200ms + 400ms + 800ms + 1.6s + 3.2s + 6.4s + 12.8s + 25.6s = ~51.1 seconds
// - Remaining 51 attempts: 51 × 30s = 1,530 seconds (25.5 minutes)
// - **Total waiting time: ~1,581 seconds (≈ 26.35 minutes)**
fn default_min_delay_ms() -> u64 {
    100
}

fn default_max_delay_ms() -> u64 {
    30_000
}

fn default_max_times() -> usize {
    60
}

fn default_factor() -> f32 {
    2.0
}

pub(crate) fn default_request_timeout_seconds() -> NonZero<u64> {
    NonZero::new(60).unwrap()
}
