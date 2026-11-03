use std::fmt;
use std::num::NonZero;
use std::time::Duration;

use schemars::JsonSchema;

/// Configuration for a single gRPC fallback endpoint.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrpcEndpointConfig {
    /// The URL of the gRPC endpoint, for example `http://fallback1:9090`.
    pub url: String,
    /// Optional authentication token, sent as `x-token` metadata.
    pub token: Option<String>,
}

impl fmt::Debug for GrpcEndpointConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrpcEndpointConfig")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "REDACTED"))
            .finish()
    }
}

/// Configuration for a single fallback (secondary) RPC endpoint.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RpcEndpointConfig {
    /// The URL of the RPC endpoint, for example `ws://fallback1:26658`.
    pub url: String,
    /// Optional JWT authentication token for this endpoint.
    pub token: Option<String>,
}

impl fmt::Debug for RpcEndpointConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcEndpointConfig")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "REDACTED"))
            .finish()
    }
}

/// Custom type matching [`celestia_rpc::TxPriority`] but with `JsonSchema` support.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TxPriority {
    #[serde(alias = "Low")]
    Low,
    #[serde(alias = "Medium")]
    Medium,
    #[serde(alias = "High")]
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

/// Controls how fetched Celestia blocks are verified against their Data Availability Header
/// before `get_block_at` returns.
///
/// Verification guards against silent consensus-breaking forks when the connected RPC node
/// returns corrupted namespace data, at the cost of per-block overhead.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VerifyOnFetchMode {
    /// Skip verification entirely.
    #[default]
    Off,
    /// Run verification; on failure log via `tracing::error!` and return the block anyway.
    /// Recommended during staged rollouts when visibility matters more than hard enforcement.
    LogError,
    /// Run verification; on failure propagate the error so `get_block_at` fails.
    ReturnError,
}

pub const fn default_safe_lead_time_ms() -> u64 {
    500
}

/// Default chunk size: the per-chunk compressed cap [`crate::envelope::MAX_COMPRESSED_CHUNK_LEN`]
/// (one signed-v1 first share minus the envelope and chunk headers). Tying the default to that cap
/// keeps a default-config chunk within a single authenticated Celestia share and guarantees the
/// uncompressed chunk size can never exceed the cap. A raw (incompressible) chunk then exactly

pub fn validate_rpc_url(rpc_url: &str) -> anyhow::Result<()> {
    if rpc_url.trim().is_empty() {
        anyhow::bail!("`rpc_url` must be set in the config or via `SOV_CELESTIA_RPC_URL`");
    }

    Ok(())
}

/// Rejects fallback endpoints with an empty/blank `url` at startup, so a misconfigured
/// `[[da.<field>]]` entry fails fast with a clear, indexed message instead of surfacing
/// later as an opaque connection error.
pub fn validate_fallback_endpoint_urls<'a>(
    field: &str,
    urls: impl Iterator<Item = &'a str>,
) -> anyhow::Result<()> {
    for (index, url) in urls.enumerate() {
        if url.trim().is_empty() {
            anyhow::bail!("`{field}[{index}].url` must not be empty");
        }
    }

    Ok(())
}

/// Validate `compression_chunk_size` against the verifier's per-chunk rollup cap.
/// An out-of-range value would otherwise be silently clamped at encode time, so a

pub fn default_rpc_url_from_env() -> String {
    std::env::var("SOV_CELESTIA_RPC_URL").unwrap_or_default()
}

pub fn default_grpc_url() -> Option<String> {
    std::env::var("SOV_CELESTIA_GRPC_URL").ok()
}

pub fn default_rpc_auth_token() -> Option<String> {
    std::env::var("SOV_CELESTIA_RPC_AUTH_TOKEN").ok()
}

pub fn default_grpc_auth_token() -> Option<String> {
    std::env::var("SOV_CELESTIA_GRPC_AUTH_TOKEN").ok()
}

pub fn default_signer_private_key() -> Option<String> {
    std::env::var("SOV_CELESTIA_SIGNER_KEY").ok()
}

/// Reads a single secondary RPC endpoint from the environment, used as the fallback
/// list when `rpc_fallback_endpoints` is omitted from the config. Both the URL and the
/// (optional) token come from the environment, since for managed providers the URL itself
/// is often a secret.
pub fn default_rpc_fallback_endpoints_from_env() -> Vec<RpcEndpointConfig> {
    match std::env::var("SOV_CELESTIA_RPC_SECONDARY_URL") {
        Ok(url) if !url.trim().is_empty() => vec![RpcEndpointConfig {
            url,
            token: std::env::var("SOV_CELESTIA_RPC_SECONDARY_AUTH_TOKEN").ok(),
        }],
        _ => Vec::new(),
    }
}

/// Reads a single secondary gRPC endpoint from the environment, used as the fallback
/// list when `grpc_fallback_endpoints` is omitted from the config.
pub fn default_grpc_fallback_endpoints_from_env() -> Vec<GrpcEndpointConfig> {
    match std::env::var("SOV_CELESTIA_GRPC_SECONDARY_URL") {
        Ok(url) if !url.trim().is_empty() => vec![GrpcEndpointConfig {
            url,
            token: std::env::var("SOV_CELESTIA_GRPC_SECONDARY_AUTH_TOKEN").ok(),
        }],
        _ => Vec::new(),
    }
}

pub const fn default_tx_priority() -> TxPriority {
    TxPriority::High
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
pub fn default_min_delay_ms() -> u64 {
    100
}

pub fn default_max_delay_ms() -> u64 {
    12_000
}

pub fn default_max_times() -> usize {
    60
}

pub fn default_factor() -> f32 {
    2.0
}

pub fn default_request_timeout_seconds() -> NonZero<u64> {
    // 6 blocks × 3 seconds + 2 seconds polling buffer
    NonZero::new(20).unwrap()
}

pub fn default_api_request_timeout_secs() -> NonZero<u64> {
    // one block time (3s) plus 2 seconds of wiggle room
    NonZero::new(5).unwrap()
}

pub fn default_tx_status_polling_millis() -> u64 {
    2_000
}

pub fn default_background_stat_polling_interval_secs() -> u64 {
    30
}

pub fn backoff_policy(
    min_delay_ms: u64,
    max_delay_ms: u64,
    max_times: usize,
    factor: f32,
) -> backon::ExponentialBuilder {
    backon::ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(min_delay_ms))
        .with_max_delay(Duration::from_millis(max_delay_ms))
        .with_max_times(max_times)
        .with_factor(factor)
}

pub struct ClientEndpoints<'a> {
    pub rpc_url: &'a str,
    pub rpc_auth_token: Option<&'a str>,
    pub rpc_fallback_endpoints: &'a [RpcEndpointConfig],
    pub rpc_health_check_interval_secs: Option<NonZero<u64>>,
    pub rpc_max_head_age_secs: Option<NonZero<u64>>,
    pub grpc_url: Option<&'a str>,
    pub grpc_auth_token: Option<&'a str>,
    pub grpc_fallback_endpoints: &'a [GrpcEndpointConfig],
    pub signer_private_key: Option<&'a str>,
    pub api_request_timeout: Duration,
}

pub async fn build_client(
    endpoints: ClientEndpoints<'_>,
    configure: impl FnOnce(celestia_client::ClientBuilder) -> celestia_client::ClientBuilder,
) -> anyhow::Result<celestia_client::Client> {
    validate_rpc_url(endpoints.rpc_url)?;
    validate_fallback_endpoint_urls(
        "rpc_fallback_endpoints",
        endpoints
            .rpc_fallback_endpoints
            .iter()
            .map(|ep| ep.url.as_str()),
    )?;
    validate_fallback_endpoint_urls(
        "grpc_fallback_endpoints",
        endpoints
            .grpc_fallback_endpoints
            .iter()
            .map(|ep| ep.url.as_str()),
    )?;
    let mut builder = celestia_client::Client::builder()
        .rpc_url(endpoints.rpc_url)
        .timeout(endpoints.api_request_timeout);
    if let Some(rpc_auth_token) = endpoints.rpc_auth_token {
        builder = builder.rpc_auth_token(rpc_auth_token);
    }
    if !endpoints.rpc_fallback_endpoints.is_empty() {
        let rpc_endpoints = endpoints.rpc_fallback_endpoints.iter().map(|ep| {
            let mut endpoint = celestia_client::RpcEndpoint::new(ep.url.clone());
            if let Some(token) = &ep.token {
                endpoint = endpoint.auth_token(token);
            }
            endpoint
        });
        builder = builder.rpc_endpoints(rpc_endpoints);
        if let Some(interval) = endpoints.rpc_health_check_interval_secs {
            builder = builder.rpc_health_check_interval(Duration::from_secs(interval.get()));
        }
        if let Some(max_head_age) = endpoints.rpc_max_head_age_secs {
            builder = builder.rpc_max_head_age(Duration::from_secs(max_head_age.get()));
        }
    }
    if endpoints.grpc_url.is_none() && !endpoints.grpc_fallback_endpoints.is_empty() {
        anyhow::bail!("`grpc_fallback_endpoints` requires `grpc_url` to be set");
    }
    if let Some(grpc_url) = endpoints.grpc_url {
        let mut endpoint = celestia_client::Endpoint::new(grpc_url.to_owned());
        if let Some(grpc_auth_token) = endpoints.grpc_auth_token {
            endpoint = endpoint.metadata("x-token", grpc_auth_token);
        }
        builder = builder.grpc_endpoint(endpoint);
        if !endpoints.grpc_fallback_endpoints.is_empty() {
            let fallback_endpoints = endpoints.grpc_fallback_endpoints.iter().map(|ep| {
                let mut endpoint = celestia_client::Endpoint::new(ep.url.clone());
                if let Some(token) = &ep.token {
                    endpoint = endpoint.metadata("x-token", token);
                }
                endpoint
            });
            builder = builder.grpc_endpoints(fallback_endpoints);
        }
        if let Some(signer_key_hex) = endpoints.signer_private_key {
            builder = builder.private_key_hex(signer_key_hex);
        }
    }
    configure(builder).build().await.map_err(Into::into)
}
