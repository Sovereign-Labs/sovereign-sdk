//! Configuration for [`crate::da_service::CelestiaService`]
use std::num::NonZero;

use schemars::JsonSchema;
use std::fmt;

/// Configuration for a single gRPC fallback endpoint.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
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

/// Runtime configuration for the [`sov_rollup_interface::node::da::DaService`] implementation.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct CelestiaConfig {
    /// The address of the Celestia RPC server
    /// For example: ws://localhost:26658
    /// Required. If not specified in the config, will be pulled from `SOV_CELESTIA_RPC_URL`.
    /// Client construction fails if both are missing.
    #[serde(default = "default_rpc_url_from_env", alias = "celestia_rpc_address")]
    pub rpc_url: String,
    /// JWT token for RPC server.
    /// If not specified in the config will be pulled from `SOV_CELESTIA_RPC_AUTH_TOKEN`
    /// environment variable.
    /// Optional.
    #[serde(default = "default_rpc_auth_token", alias = "celestia_rpc_auth_token")]
    pub rpc_auth_token: Option<String>,

    /// The address of the Celestia gRPC server, for example, http://localhost:9090
    /// If not specified in the config, will be pulled from `SOV_CELESTIA_GRPC_URL`.
    /// Optional.
    /// Set only if DaService needs to submit blobs.
    #[serde(default = "default_grpc_url")]
    pub grpc_url: Option<String>,

    /// The token for accessing Celestia gRPC server.
    /// If not specified in the config, will be pulled from `SOV_CELESTIA_GRPC_AUTH_TOKEN`.
    /// Optional, used only if `grpc_url` is set.
    #[serde(default = "default_grpc_auth_token")]
    pub grpc_auth_token: Option<String>,

    /// Optional list of fallback gRPC endpoints for blob submission.
    /// Used alongside `grpc_url` for failover. Each entry has a mandatory `url`
    /// and an optional `token` (sent as `x-token` metadata).
    /// Default: empty (no fallback endpoints).
    #[serde(default)]
    pub grpc_fallback_endpoints: Vec<GrpcEndpointConfig>,

    /// The private key in hex format of Celestia wallet that has enough TIA to publish blobs.
    /// If not specified in the config, will be pulled from `SOV_CELESTIA_SIGNER_KEY`.
    /// Can be exported from `celestia-appd`:
    /// `celestia-appd keys export --unsafe --unarmored-hex key-name --keyring-backend test`
    /// Or from `cel-key` (light/bridge node):
    /// `cel-key export key-name --unarmored-hex --unsafe --node.type light --p2p.network=mocha`
    #[serde(default = "default_signer_private_key")]
    pub signer_private_key: Option<String>,
    /// High-level timeout for Celestia RPC operations that may include multiple requests (in seconds).
    /// Default: 38 (6 blocks × 6 seconds + 2 seconds polling buffer).
    #[serde(
        default = "default_request_timeout_seconds",
        alias = "celestia_rpc_timeout_seconds"
    )]
    pub request_timeout_secs: NonZero<u64>,
    /// Timeout for individual API requests to the Celestia node (in seconds).
    /// This is passed to the underlying celestia-client for each API call.
    /// Default: 8 (one block time plus 2 seconds of wiggle room).
    #[serde(default = "default_api_request_timeout_secs")]
    pub api_request_timeout_secs: NonZero<u64>,
    /// Interval for polling transaction status confirmation (in milliseconds).
    /// Default: 2000.
    #[serde(default = "default_tx_status_polling_millis")]
    pub tx_status_polling_millis: u64,
    /// Interval for background statistics collection (in seconds).
    /// Set to 0 to disable the background stat collection task.
    /// Default: 30.
    #[serde(default = "default_background_stat_polling_interval_secs")]
    pub background_stat_polling_interval_secs: u64,
    /// Controls how fetched Celestia blocks are verified against their Data Availability
    /// Header before `get_block_at` returns. See [`VerifyOnFetchMode`].
    #[serde(default)]
    pub verify_on_fetch_mode: VerifyOnFetchMode,
    /// See [`sov_rollup_interface::node::da::DaService::safe_lead_time`].
    #[serde(default = "default_safe_lead_time_ms")]
    pub safe_lead_time_ms: u64,

    /// Default is high.
    #[serde(default = "default_tx_priority")]
    pub tx_priority: TxPriority,
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

    /// Whether to compress batch blobs before submission. Emission only — never
    /// affects how blobs are read or verified. Default: `Off`.
    #[serde(default)]
    pub compression: CompressOnSubmit,
    /// Target chunk size (uncompressed bytes) for the chunked compression envelope.
    /// Must be in `1..=MAX_ROLLUP_CHUNK_LEN` (16384 by default — the verifier's per-chunk
    /// rollup cap); an out-of-range value is rejected at startup rather than silently clamped.
    /// Default: 482 (one continuation share payload, share-aligned). Advanced knob — larger
    /// chunks trade coarser partial-read granularity for a better ratio, but a chunk that does
    /// not compress below the per-chunk compressed cap (`MAX_COMPRESSED_CHUNK_LEN`) makes the
    /// envelope non-canonical, so that batch is posted verbatim (uncompressed), not rejected.
    #[serde(default = "default_compression_chunk_size")]
    pub compression_chunk_size: usize,
}

impl fmt::Debug for CelestiaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CelestiaConfig")
            .field("rpc_url", &self.rpc_url)
            .field(
                "rpc_auth_token",
                &self.rpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field("grpc_url", &self.grpc_url)
            .field(
                "grpc_auth_token",
                &self.grpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field("grpc_fallback_endpoints", &self.grpc_fallback_endpoints)
            .field(
                "signer_private_key",
                &self.signer_private_key.as_ref().map(|_| "REDACTED"),
            )
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("api_request_timeout_secs", &self.api_request_timeout_secs)
            .field("tx_status_polling_millis", &self.tx_status_polling_millis)
            .field(
                "background_stat_polling_interval_secs",
                &self.background_stat_polling_interval_secs,
            )
            .field("verify_on_fetch_mode", &self.verify_on_fetch_mode)
            .field("safe_lead_time_ms", &self.safe_lead_time_ms)
            .field("tx_priority", &self.tx_priority)
            .field("backoff_min_delay_ms", &self.backoff_min_delay_ms)
            .field("backoff_max_delay_ms", &self.backoff_max_delay_ms)
            .field("backoff_max_times", &self.backoff_max_times)
            .field("backoff_factor", &self.backoff_factor)
            .field("compression", &self.compression)
            .field("compression_chunk_size", &self.compression_chunk_size)
            .finish()
    }
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

/// Whether the adapter compresses batch blobs before posting them to Celestia.
///
/// Controls **emission only** — read and verification semantics never depend on it.
/// Operators must keep it `Off` until every node/prover on the network can read
/// compression envelopes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize, JsonSchema,
)]
pub enum CompressOnSubmit {
    /// Post batch blobs verbatim (today's behavior).
    #[default]
    Off,
    /// Wrap batch blobs in a chunked LZ4 envelope when it is strictly smaller than raw.
    Lz4,
}

impl CompressOnSubmit {
    pub(crate) fn is_enabled(&self) -> bool {
        match self {
            CompressOnSubmit::Off => false,
            CompressOnSubmit::Lz4 => true,
        }
    }
}

impl fmt::Display for CompressOnSubmit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompressOnSubmit::Off => f.write_str("off"),
            CompressOnSubmit::Lz4 => f.write_str("lz4"),
        }
    }
}

impl CelestiaConfig {
    /// Absolutely minimal config for client that is capable of reading
    pub fn minimal(rpc_url: String) -> Self {
        Self {
            rpc_url,
            rpc_auth_token: None,
            grpc_url: None,
            grpc_auth_token: None,
            grpc_fallback_endpoints: Vec::new(),
            signer_private_key: None,
            request_timeout_secs: default_request_timeout_seconds(),
            api_request_timeout_secs: default_api_request_timeout_secs(),
            tx_status_polling_millis: default_tx_status_polling_millis(),
            background_stat_polling_interval_secs: default_background_stat_polling_interval_secs(),
            verify_on_fetch_mode: VerifyOnFetchMode::default(),
            safe_lead_time_ms: default_safe_lead_time_ms(),
            tx_priority: default_tx_priority(),
            backoff_min_delay_ms: default_min_delay_ms(),
            backoff_max_delay_ms: default_max_delay_ms(),
            backoff_max_times: default_max_times(),
            backoff_factor: default_factor(),
            compression: CompressOnSubmit::default(),
            compression_chunk_size: default_compression_chunk_size(),
        }
    }

    /// Add necessary information required for submitting blobs
    pub fn with_submission(mut self, grpc_url: String, signer_private_key: String) -> Self {
        self.grpc_url = Some(grpc_url);
        self.signer_private_key = Some(signer_private_key);
        self
    }

    pub(crate) fn get_backoff_policy(&self) -> backon::ExponentialBuilder {
        let backoff_policy = backon::ExponentialBuilder::default()
            .with_min_delay(std::time::Duration::from_millis(self.backoff_min_delay_ms))
            .with_max_times(self.backoff_max_times)
            .with_max_delay(std::time::Duration::from_millis(self.backoff_max_delay_ms))
            .with_factor(self.backoff_factor);

        tracing::debug!(?backoff_policy, "Configured backoff policy");
        backoff_policy
    }

    pub(crate) async fn build_client(&self) -> anyhow::Result<celestia_client::Client> {
        validate_rpc_url(&self.rpc_url)?;
        validate_compression_chunk_size(self.compression_chunk_size)?;

        let api_request_timeout =
            std::time::Duration::from_secs(self.api_request_timeout_secs.get());
        let mut builder = celestia_client::Client::builder()
            .rpc_url(&self.rpc_url)
            .timeout(api_request_timeout);

        if let Some(rpc_auth_token) = &self.rpc_auth_token {
            builder = builder.rpc_auth_token(rpc_auth_token);
        }
        // Submission section.
        if self.grpc_url.is_none() && !self.grpc_fallback_endpoints.is_empty() {
            anyhow::bail!("`grpc_fallback_endpoints` requires `grpc_url` to be set");
        }
        if let Some(grpc_url) = &self.grpc_url {
            let mut endpoint = celestia_client::Endpoint::new(grpc_url.clone());
            if let Some(grpc_auth_token) = &self.grpc_auth_token {
                endpoint = endpoint.metadata("x-token", grpc_auth_token);
            }
            builder = builder.grpc_endpoint(endpoint);
            if !self.grpc_fallback_endpoints.is_empty() {
                let fallback_endpoints = self.grpc_fallback_endpoints.iter().map(|ep| {
                    let mut endpoint = celestia_client::Endpoint::new(ep.url.clone());
                    if let Some(token) = &ep.token {
                        endpoint = endpoint.metadata("x-token", token);
                    }
                    endpoint
                });
                builder = builder.grpc_endpoints(fallback_endpoints);
            }
            if let Some(signer_key_hex) = &self.signer_private_key {
                builder = builder.private_key_hex(signer_key_hex);
            }
        }
        builder.build().await.map_err(Into::into)
    }
}

pub(crate) const fn default_safe_lead_time_ms() -> u64 {
    500
}

/// Default chunk size: one continuation sparse-share payload, so the default tracks
/// Celestia's share geometry if the share size ever changes. Pinned to one share — not
/// the per-chunk cap [`crate::envelope::MAX_ROLLUP_CHUNK_LEN`] — so the conservative
/// default does not move when the cap is raised.
pub(crate) const fn default_compression_chunk_size() -> usize {
    celestia_types::consts::appconsts::CONTINUATION_SPARSE_SHARE_CONTENT_SIZE
}

fn validate_rpc_url(rpc_url: &str) -> anyhow::Result<()> {
    if rpc_url.trim().is_empty() {
        anyhow::bail!("`rpc_url` must be set in the config or via `SOV_CELESTIA_RPC_URL`");
    }

    Ok(())
}

/// Validate `compression_chunk_size` against the verifier's per-chunk rollup cap.
/// An out-of-range value would otherwise be silently clamped at encode time, so a
/// misconfigured node would quietly emit different chunks than the operator requested.
fn validate_compression_chunk_size(chunk_size: usize) -> anyhow::Result<()> {
    let max = crate::envelope::MAX_ROLLUP_CHUNK_LEN as usize;
    if !(1..=max).contains(&chunk_size) {
        anyhow::bail!(
            "`compression_chunk_size` must be between 1 and {max} (the verifier's per-chunk cap), got {chunk_size}"
        );
    }

    Ok(())
}

fn default_rpc_url_from_env() -> String {
    std::env::var("SOV_CELESTIA_RPC_URL").unwrap_or_default()
}

fn default_grpc_url() -> Option<String> {
    std::env::var("SOV_CELESTIA_GRPC_URL").ok()
}

fn default_rpc_auth_token() -> Option<String> {
    std::env::var("SOV_CELESTIA_RPC_AUTH_TOKEN").ok()
}

fn default_grpc_auth_token() -> Option<String> {
    std::env::var("SOV_CELESTIA_GRPC_AUTH_TOKEN").ok()
}

fn default_signer_private_key() -> Option<String> {
    std::env::var("SOV_CELESTIA_SIGNER_KEY").ok()
}

pub(crate) const fn default_tx_priority() -> TxPriority {
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
pub(crate) fn default_min_delay_ms() -> u64 {
    100
}

pub(crate) fn default_max_delay_ms() -> u64 {
    12_000
}

pub(crate) fn default_max_times() -> usize {
    60
}

pub(crate) fn default_factor() -> f32 {
    2.0
}

pub(crate) fn default_request_timeout_seconds() -> NonZero<u64> {
    // 6 blocks × 6 seconds + 2 seconds polling buffer
    NonZero::new(38).unwrap()
}

pub(crate) fn default_api_request_timeout_secs() -> NonZero<u64> {
    NonZero::new(8).unwrap()
}

pub(crate) fn default_tx_status_polling_millis() -> u64 {
    2_000
}

pub(crate) fn default_background_stat_polling_interval_secs() -> u64 {
    30
}

#[cfg(test)]
mod tests {
    use super::{
        default_compression_chunk_size, validate_compression_chunk_size, validate_rpc_url,
        CelestiaConfig, GrpcEndpointConfig, VerifyOnFetchMode,
    };

    const RPC_ENV_VAR: &str = "SOV_CELESTIA_RPC_URL";
    const GRPC_ENV_VAR: &str = "SOV_CELESTIA_GRPC_URL";

    struct EnvVarGuard {
        key: &'static str,
        previous_value: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let previous_value = std::env::var(key).ok();
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
            Self {
                key,
                previous_value,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous_value {
                Some(previous_value) => std::env::set_var(self.key, previous_value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn deserialize_config(json: &str) -> Result<CelestiaConfig, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn rpc_url_is_read_from_env_when_missing_from_config() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

        let config = deserialize_config("{}").unwrap();
        assert_eq!(config.rpc_url, "ws://env-rpc:26658");
    }

    #[test]
    fn rpc_url_validation_fails_when_missing_from_config_and_env() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, None);
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

        let config = deserialize_config("{}").unwrap();
        let error = validate_rpc_url(&config.rpc_url).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`rpc_url` must be set in the config or via `SOV_CELESTIA_RPC_URL`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn explicit_rpc_url_overrides_env() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

        let config = deserialize_config(r#"{"rpc_url":"ws://config-rpc:26658"}"#).unwrap();
        assert_eq!(config.rpc_url, "ws://config-rpc:26658");
    }

    #[test]
    fn grpc_url_is_read_from_env_when_missing_from_config() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, Some("http://env-grpc:9090"));

        let config = deserialize_config("{}").unwrap();
        assert_eq!(config.grpc_url.as_deref(), Some("http://env-grpc:9090"));
    }

    #[test]
    fn grpc_url_is_none_when_missing_from_config_and_env() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

        let config = deserialize_config("{}").unwrap();
        assert_eq!(config.grpc_url, None);
    }

    #[test]
    fn explicit_grpc_url_overrides_env() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, Some("http://env-grpc:9090"));

        let config = deserialize_config(
            r#"{"rpc_url":"ws://config-rpc:26658","grpc_url":"http://config-grpc:9090"}"#,
        )
        .unwrap();
        assert_eq!(config.grpc_url.as_deref(), Some("http://config-grpc:9090"));
    }

    #[test]
    fn grpc_fallback_endpoints_default_to_empty() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

        let config = deserialize_config("{}").unwrap();
        assert!(config.grpc_fallback_endpoints.is_empty());
    }

    #[test]
    fn grpc_fallback_endpoints_with_token_deserialize() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

        let config = deserialize_config(
            r#"{
                "grpc_fallback_endpoints": [
                    {"url": "http://fallback1:9090", "token": "secret-token"},
                    {"url": "http://fallback2:9090"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(config.grpc_fallback_endpoints.len(), 2);
        assert_eq!(
            config.grpc_fallback_endpoints[0].url,
            "http://fallback1:9090"
        );
        assert_eq!(
            config.grpc_fallback_endpoints[0].token.as_deref(),
            Some("secret-token"),
        );
        assert_eq!(
            config.grpc_fallback_endpoints[1].url,
            "http://fallback2:9090"
        );
        assert_eq!(config.grpc_fallback_endpoints[1].token, None);
    }

    #[test]
    fn debug_redacts_fallback_endpoint_tokens() {
        let mut config = CelestiaConfig::minimal("ws://rpc:26658".to_string());
        config.grpc_fallback_endpoints = vec![
            GrpcEndpointConfig {
                url: "http://fallback1:9090".to_string(),
                token: Some("super-secret".to_string()),
            },
            GrpcEndpointConfig {
                url: "http://fallback2:9090".to_string(),
                token: None,
            },
        ];
        let debug_output = format!("{:?}", config);
        assert!(
            !debug_output.contains("super-secret"),
            "token should be redacted in debug output: {debug_output}"
        );
        assert!(debug_output.contains("REDACTED"));
        assert!(debug_output.contains("http://fallback1:9090"));
        assert!(debug_output.contains("http://fallback2:9090"));
    }

    #[tokio::test]
    async fn build_client_rejects_fallback_endpoints_without_grpc_url() {
        let mut config = CelestiaConfig::minimal("ws://localhost:26658".to_string());
        config.grpc_fallback_endpoints = vec![GrpcEndpointConfig {
            url: "http://fallback:9090".to_string(),
            token: None,
        }];
        let error = config.build_client().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`grpc_fallback_endpoints` requires `grpc_url` to be set"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn grpc_fallback_endpoints_roundtrip() {
        let mut config = CelestiaConfig::minimal("ws://rpc:26658".to_string());
        config.grpc_fallback_endpoints = vec![GrpcEndpointConfig {
            url: "http://fallback1:9090".to_string(),
            token: Some("token1".to_string()),
        }];
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: CelestiaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn verify_on_fetch_mode_defaults_to_off() {
        let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
        let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

        let config = deserialize_config("{}").unwrap();
        assert_eq!(config.verify_on_fetch_mode, VerifyOnFetchMode::Off);
    }

    #[test]
    fn verify_on_fetch_mode_accepts_all_variants() {
        let cases = [
            (r#"{"verify_on_fetch_mode":"off"}"#, VerifyOnFetchMode::Off),
            (
                r#"{"verify_on_fetch_mode":"log_error"}"#,
                VerifyOnFetchMode::LogError,
            ),
            (
                r#"{"verify_on_fetch_mode":"return_error"}"#,
                VerifyOnFetchMode::ReturnError,
            ),
        ];
        for (json, expected) in cases {
            let _rpc_guard = EnvVarGuard::set(RPC_ENV_VAR, Some("ws://env-rpc:26658"));
            let _grpc_guard = EnvVarGuard::set(GRPC_ENV_VAR, None);

            let config = deserialize_config(json).expect(json);
            assert_eq!(config.verify_on_fetch_mode, expected);
        }
    }

    #[test]
    fn compression_chunk_size_in_range_is_accepted() {
        let max = crate::envelope::MAX_ROLLUP_CHUNK_LEN as usize;
        assert!(validate_compression_chunk_size(1).is_ok());
        assert!(validate_compression_chunk_size(default_compression_chunk_size()).is_ok());
        assert!(validate_compression_chunk_size(max).is_ok());
    }

    #[test]
    fn compression_chunk_size_out_of_range_is_rejected() {
        let max = crate::envelope::MAX_ROLLUP_CHUNK_LEN as usize;
        assert!(validate_compression_chunk_size(0).is_err());
        assert!(validate_compression_chunk_size(max + 1).is_err());
    }

    #[test]
    fn default_compression_chunk_size_is_one_continuation_share() {
        // Guards against re-coupling the default to the per-chunk cap.
        assert_eq!(default_compression_chunk_size(), 482);
    }
}
