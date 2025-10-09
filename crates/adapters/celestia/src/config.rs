//! Configuration for [`crate::da_service::CelestiaService`]
use crate::verifier::address::CelestiaAddress;
use jsonrpsee::http_client::{HeaderMap, HttpClientBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serializer};
use std::num::NonZero;
use std::time::Duration;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "-", env!("CARGO_PKG_VERSION"));

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
    pub twinkle: Option<TwinkleConfig>,
}

/// Custom type matching [`celestia_rpc::TxPriority`] but with `JsonSchema` support.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub enum TxPriority {
    Low,
    Medium,
    High,
}

impl From<TxPriority> for celestia_rpc::TxPriority {
    fn from(value: TxPriority) -> Self {
        match value {
            TxPriority::Low => celestia_rpc::TxPriority::Low,
            TxPriority::Medium => celestia_rpc::TxPriority::Medium,
            TxPriority::High => celestia_rpc::TxPriority::High,
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
            celestia_rpc_auth_token: "TEST".to_string(),
            celestia_rpc_address: url.to_string(),
            max_celestia_response_body_size: NonZero::new(1024 * 1024 * 100).unwrap(),
            celestia_rpc_timeout_seconds: NonZero::new(120).unwrap(),
            safe_lead_time_ms: 500,
            signer_address: None,
            tx_priority: None,
            backoff_min_delay_ms: 50,
            backoff_max_delay_ms: 100,
            backoff_max_times: 3,
            backoff_factor: default_factor(),
            twinkle: None,
        }
    }

    pub(crate) fn request_timeout(&self) -> std::time::Duration {
        Duration::from_secs(self.celestia_rpc_timeout_seconds.get())
    }

    pub(crate) fn construct_rpc_client(&self) -> jsonrpsee::http_client::HttpClient {
        {
            let mut headers = HeaderMap::new();
            headers.insert(
                "Authorization",
                format!("Bearer {}", self.celestia_rpc_auth_token)
                    .parse()
                    .unwrap(),
            );

            HttpClientBuilder::default()
                .set_headers(headers)
                .max_response_size(self.max_celestia_response_body_size.get())
                .max_request_size(self.max_celestia_response_body_size.get())
                .request_timeout(self.request_timeout())
                .build(&self.celestia_rpc_address)
        }
        .expect("RPC HttpClient initialization should be valid")
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

#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct TwinkleConfig {
    /// If not set, will be taken from the environment variable `SOV_TWINKLE_API_KEY`
    pub api_key: Option<String>,
    /// Celestia network: "mocha" or "mainnet"
    pub network: Network,
    /// At which interval pull blob status after it has been submitted.
    #[serde(default = "default_pull_interval_millis")]
    pub pull_interval_millis: u64,
    /// Timeout for individual HTTP requests in seconds
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// Timeout for the entire request including retries in seconds
    // TODO:
    pub total_timeout_secs: u64,
    /// Timeout for establishing HTTP connections in seconds
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    /// Timeout for idle connections in the connection pool in seconds
    #[serde(default = "default_pool_idle_timeout_secs")]
    pub pool_idle_timeout_secs: u64,
}

impl std::fmt::Debug for TwinkleConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_api_key = self.api_key.as_ref().map(|_| "<redacted>");
        f.debug_struct("TwinkleConfig")
            .field("api_key", &redacted_api_key)
            .field("network", &self.network)
            .field("pull_interval_millis", &self.pull_interval_millis)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("total_timeout_secs", &self.total_timeout_secs)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field("pool_idle_timeout_secs", &self.pool_idle_timeout_secs)
            .finish()
    }
}

fn default_pull_interval_millis() -> u64 {
    100
}

fn default_request_timeout_secs() -> u64 {
    60
}

fn default_connect_timeout_secs() -> u64 {
    10
}

fn default_pool_idle_timeout_secs() -> u64 {
    30
}

impl TwinkleConfig {
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            api_key: Some("TEMP_SECRET".to_string()),
            network: Network::Mocha,
            pull_interval_millis: default_pull_interval_millis(),
            request_timeout_secs: default_request_timeout_secs(),
            // TODO: Use same as in CelestiaConfig.
            total_timeout_secs: 120,
            connect_timeout_secs: default_connect_timeout_secs(),
            pool_idle_timeout_secs: default_pool_idle_timeout_secs(),
        }
    }
    fn api_key(&self) -> String {
        match &self.api_key {
            None => std::env::var("SOV_TWINKLE_API_KEY").expect(
                "SOV_TWINKLE_API_KEY environment is not set, cannot initialize TwinkleClient",
            ),
            Some(set) => set.clone(),
        }
    }

    pub fn construct_reqwest_client(&self) -> reqwest::Client {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut auth_value =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.api_key()))
                .expect("Failed to set TwinkleClient auth");
        auth_value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static(USER_AGENT),
        );

        reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(self.request_timeout_secs))
            .connect_timeout(std::time::Duration::from_secs(self.connect_timeout_secs))
            .pool_idle_timeout(std::time::Duration::from_secs(self.pool_idle_timeout_secs))
            .build()
            .expect("Reqwest HTTP client config should be valid")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mocha,
    Mainnet,
}

impl Network {
    fn as_str(&self) -> &str {
        match self {
            Network::Mocha => "mocha-4",
            Network::Mainnet => "mainnet",
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for Network {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}
