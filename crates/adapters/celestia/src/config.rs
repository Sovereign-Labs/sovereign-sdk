//! Configuration for [`crate::da_service::CelestiaService`]
use std::num::NonZero;

use schemars::JsonSchema;

/// Runtime configuration for the [`sov_rollup_interface::node::da::DaService`] implementation.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CelestiaConfig {
    /// The address of the Celestia RPC server
    /// ws://localhost:26658
    #[serde(default = "default_rpc_addr", alias = "celestia_rpc_address")]
    pub rpc_url: String,
    /// TODO: Docs
    #[serde(alias = "celestia_rpc_auth_token")]
    pub rpc_auth_token: Option<String>,

    /// http://localhost:9090
    pub grpc_url: Option<String>,

    /// TODO
    pub grpc_auth_token: Option<String>,

    /// Now hex, later seed phrase and file.
    pub signer_private_key: Option<String>,
    /// The timeout for a Celestia RPC request, in seconds.
    /// TODO: Currently unused, because celestia client does not expose such params
    #[serde(
        default = "default_request_timeout_seconds",
        alias = "celestia_rpc_timeout_seconds"
    )]
    pub request_timeout_secs: NonZero<u64>,
    /// See [`sov_rollup_interface::node::da::DaService::safe_lead_time`].
    #[serde(default = "default_safe_lead_time_ms")]
    pub safe_lead_time_ms: u64,

    /// Default is medium.
    pub tx_priority: Option<TxPriority>,
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

/// Custom type matching [`celestia_rpc::TxPriority`] but with `JsonSchema` support.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub enum TxPriority {
    Low,
    Medium,
    High,
}

impl From<TxPriority> for celestia_client::tx::TxPriority {
    fn from(value: TxPriority) -> Self {
        match value {
            TxPriority::Low => celestia_client::tx::TxPriority::Low,
            TxPriority::Medium => celestia_client::tx::TxPriority::Medium,
            TxPriority::High => celestia_client::tx::TxPriority::High,
        }
    }
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
            rpc_auth_token: None,
            grpc_auth_token: None,
            rpc_url: url.to_string(),
            grpc_url: None,
            signer_private_key: None,
            request_timeout_secs: NonZero::new(120).unwrap(),
            safe_lead_time_ms: 500,
            tx_priority: None,
            backoff_min_delay_ms: 50,
            backoff_max_delay_ms: 100,
            backoff_max_times: 3,
            backoff_factor: default_factor(),
        }
    }

    /// Gets auth token from parsed config or from `SOV_CELESTIA_GRPC_AUTH_TOKEN` environment variable.
    /// Returns None if both are unset.
    /// Config value has priority over environment variable.
    fn grpc_auth_token(&self) -> Option<String> {
        self.grpc_auth_token
            .clone()
            .or_else(|| std::env::var("SOV_CELESTIA_GRPC_AUTH_TOKEN").ok())
    }

    /// Gets auth token from parsed config or from `SOV_CELESTIA_RPC_AUTH_TOKEN` environment variable
    /// Returns None if both are unset.
    /// Config value has priority over environment variable.
    fn rpc_auth_token(&self) -> Option<String> {
        self.rpc_auth_token
            .clone()
            .or_else(|| std::env::var("SOV_CELESTIA_RPC_AUTH_TOKEN").ok())
    }

    pub(crate) async fn build_client(&self) -> anyhow::Result<celestia_client::Client> {
        let mut builder = celestia_client::Client::builder().rpc_url(&self.rpc_url);
        if let Some(rpc_auth_token) = self.rpc_auth_token() {
            builder = builder.rpc_auth_token(&rpc_auth_token);
        }
        // Submission section.
        if let Some(grpc_url) = &self.grpc_url {
            builder = builder.grpc_url(grpc_url);
        }
        if let Some(grpc_auth_token) = self.grpc_auth_token() {
            builder = builder.grpc_metadata("x-token", &grpc_auth_token);
        }
        if let Some(signer_key_hex) = &self.signer_private_key {
            builder = builder.private_key_hex(signer_key_hex);
        }

        builder.build().await.map_err(Into::into)
    }
}

fn default_safe_lead_time_ms() -> u64 {
    500
}

fn default_rpc_addr() -> String {
    "http://localhost:11111/".into()
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
// 10. Attempt 10: 10s (capped at max_delay)
// 11. Attempt 11-60: 10s each
// **Total Number of Attempts:** 60 (as specified by ) `with_max_times(60)`
// **Total Waiting Time:**
// - First 9 attempts: 100ms + 200ms + 400ms + 800ms + 1.6s + 3.2s + 6.4s + 12.8s + 25.6s = ~51.1 seconds
// - Remaining 51 attempts: 51 × 30s = 1,530 seconds (25.5 minutes)
// - **Total waiting time: ~1,581 seconds (≈ 26.35 minutes)**
fn default_min_delay_ms() -> u64 {
    100
}

fn default_max_delay_ms() -> u64 {
    12_000
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
