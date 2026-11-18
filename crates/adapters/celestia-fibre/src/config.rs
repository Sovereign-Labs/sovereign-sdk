use std::fmt;
use std::num::NonZero;
use std::time::Duration;

use sov_celestia_adapter::{GrpcEndpointConfig, RpcEndpointConfig};

/// Runtime configuration for [`crate::CelestiaFibreDaService`].
///
/// Parameters shared with `sov_celestia_adapter::CelestiaConfig` have the same names, meaning,
/// defaults and environment variables.
///
/// There is no `verify_on_fetch_mode`: `get_block_at` always verifies the namespace data against
/// the block header, because it builds the namespace proof from it.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CelestiaFibreConfig {
    /// Celestia bridge node RPC, for example `ws://localhost:26658`.
    /// Serves headers and the v2 shares that reference Fibre blobs.
    /// If not specified in the config, it is read from `SOV_CELESTIA_RPC_URL`.
    /// Client construction fails if both are missing.
    #[serde(default = "default_rpc_url_from_env")]
    pub rpc_url: String,
    /// JWT token for the RPC server.
    /// If not specified in the config, it is read from `SOV_CELESTIA_RPC_AUTH_TOKEN`.
    #[serde(default = "default_rpc_auth_token")]
    pub rpc_auth_token: Option<String>,
    /// Fallback RPC endpoints, tried in order when `rpc_url` is unreachable. The client switches
    /// back to `rpc_url` once it recovers.
    /// If omitted from the config, a single fallback endpoint is read from
    /// `SOV_CELESTIA_RPC_SECONDARY_URL` and `SOV_CELESTIA_RPC_SECONDARY_AUTH_TOKEN`.
    #[serde(default = "default_rpc_fallback_endpoints_from_env")]
    pub rpc_fallback_endpoints: Vec<RpcEndpointConfig>,
    /// How often the preferred RPC endpoint is probed to switch back to it (in seconds).
    /// Only used with `rpc_fallback_endpoints`. Default: the celestia-client built-in value.
    #[serde(default)]
    pub rpc_health_check_interval_secs: Option<NonZero<u64>>,
    /// Maximum age of the preferred RPC endpoint's head for switching back to it (in seconds).
    /// Only used with `rpc_fallback_endpoints`. Default: the celestia-client built-in value.
    #[serde(default)]
    pub rpc_max_head_age_secs: Option<NonZero<u64>>,
    /// Celestia consensus node gRPC, for example `http://localhost:9090`.
    /// Fibre needs it for both downloads (validator set) and submissions.
    /// If not specified in the config, it is read from `SOV_CELESTIA_GRPC_URL`.
    /// Client construction fails if both are missing.
    #[serde(default = "default_grpc_url_from_env")]
    pub grpc_url: String,
    /// Optional gRPC authentication token, sent as `x-token` metadata.
    /// If not specified in the config, it is read from `SOV_CELESTIA_GRPC_AUTH_TOKEN`.
    #[serde(default = "default_grpc_auth_token")]
    pub grpc_auth_token: Option<String>,
    /// Fallback gRPC endpoints, tried in order on network errors with `grpc_url`. Each entry has
    /// an optional `token`, sent as `x-token` metadata.
    /// If omitted from the config, a single fallback endpoint is read from
    /// `SOV_CELESTIA_GRPC_SECONDARY_URL` and `SOV_CELESTIA_GRPC_SECONDARY_AUTH_TOKEN`.
    #[serde(default = "default_grpc_fallback_endpoints_from_env")]
    pub grpc_fallback_endpoints: Vec<GrpcEndpointConfig>,
    /// Consensus node gRPC used only for Fibre's validator-set and validator-host queries, for
    /// when the `grpc_url` endpoints don't serve them. Default: the `grpc_url` endpoints.
    #[serde(default)]
    pub fibre_grpc_url: Option<String>,
    /// Authentication token for `fibre_grpc_url`, sent as `x-token` metadata.
    #[serde(default)]
    pub fibre_grpc_auth_token: Option<String>,
    /// Hex-encoded secp256k1 private key. Without it the service is read-only.
    /// If not specified in the config, it is read from `SOV_CELESTIA_SIGNER_KEY`.
    #[serde(default = "default_signer_private_key")]
    pub signer_private_key: Option<String>,
    /// Timeout for one request to a single Fibre validator: a shard download or a shard upload,
    /// connection setup included (in seconds). A validator that exceeds it is treated as failed,
    /// and the client asks another validator. Must allow uploading a full shard. Default: 30, as
    /// in `celestia-grpc`.
    #[serde(default = "default_fibre_request_timeout_secs")]
    pub fibre_request_timeout_secs: NonZero<u64>,
    /// Timeout for one attempt of a Celestia node operation, which may include multiple
    /// requests (in seconds). Each attempt is retried according to the `backoff_*` parameters.
    /// Default: 20 (6 blocks × 3 seconds + 2 seconds polling buffer).
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: NonZero<u64>,
    /// Timeout for individual RPC and gRPC requests to the Celestia nodes (in seconds).
    /// Default: 5 (one block time (3s) plus 2 seconds of wiggle room).
    #[serde(default = "default_api_request_timeout_secs")]
    pub api_request_timeout_secs: NonZero<u64>,
    /// Minimal time to wait before retrying a failed request to a Celestia node (in milliseconds).
    /// See [`backon::ExponentialBuilder`] for more details. Default: 100.
    #[serde(default = "default_backoff_min_delay_ms")]
    pub backoff_min_delay_ms: u64,
    /// Maximal time between retries of a failed request to a Celestia node (in milliseconds).
    /// See [`backon::ExponentialBuilder`] for more details. Default: 12000.
    #[serde(default = "default_backoff_max_delay_ms")]
    pub backoff_max_delay_ms: u64,
    /// Number of retries of a failed request to a Celestia node before returning an error.
    /// See [`backon::ExponentialBuilder`] for more details. Default: 60.
    #[serde(default = "default_backoff_max_times")]
    pub backoff_max_times: usize,
    /// Exponential factor between retries of a failed request to a Celestia node.
    /// See [`backon::ExponentialBuilder`] for more details. Default: 2.0.
    #[serde(default = "default_backoff_factor")]
    pub backoff_factor: f32,
}

impl CelestiaFibreConfig {
    pub(crate) fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs.get())
    }

    pub(crate) fn api_request_timeout(&self) -> Duration {
        Duration::from_secs(self.api_request_timeout_secs.get())
    }

    pub(crate) fn fibre_request_timeout(&self) -> Duration {
        Duration::from_secs(self.fibre_request_timeout_secs.get())
    }

    pub(crate) fn backoff_policy(&self) -> backon::ExponentialBuilder {
        backon::ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(self.backoff_min_delay_ms))
            .with_max_times(self.backoff_max_times)
            .with_max_delay(Duration::from_millis(self.backoff_max_delay_ms))
            .with_factor(self.backoff_factor)
    }

    /// Rejects missing or blank endpoint URLs at startup with a clear message, instead of an
    /// opaque connection error later.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        if self.rpc_url.trim().is_empty() {
            anyhow::bail!("`rpc_url` must be set in the config or via `SOV_CELESTIA_RPC_URL`");
        }
        if self.grpc_url.trim().is_empty() {
            anyhow::bail!("`grpc_url` must be set in the config or via `SOV_CELESTIA_GRPC_URL`");
        }
        let fallback_urls = self
            .rpc_fallback_endpoints
            .iter()
            .map(|endpoint| ("rpc_fallback_endpoints", &endpoint.url))
            .chain(
                self.grpc_fallback_endpoints
                    .iter()
                    .map(|endpoint| ("grpc_fallback_endpoints", &endpoint.url)),
            );
        for (index, (field, url)) in fallback_urls.enumerate() {
            if url.trim().is_empty() {
                anyhow::bail!("`{field}` entry {index} has an empty `url`");
            }
        }
        if self
            .fibre_grpc_url
            .as_ref()
            .is_some_and(|url| url.trim().is_empty())
        {
            anyhow::bail!("`fibre_grpc_url` must not be empty when set");
        }
        Ok(())
    }
}

// Defaults and environment variables below match `sov_celestia_adapter::CelestiaConfig`; keep
// them in sync.

fn default_rpc_url_from_env() -> String {
    std::env::var("SOV_CELESTIA_RPC_URL").unwrap_or_default()
}

fn default_rpc_auth_token() -> Option<String> {
    std::env::var("SOV_CELESTIA_RPC_AUTH_TOKEN").ok()
}

fn default_grpc_url_from_env() -> String {
    std::env::var("SOV_CELESTIA_GRPC_URL").unwrap_or_default()
}

fn default_grpc_auth_token() -> Option<String> {
    std::env::var("SOV_CELESTIA_GRPC_AUTH_TOKEN").ok()
}

fn default_signer_private_key() -> Option<String> {
    std::env::var("SOV_CELESTIA_SIGNER_KEY").ok()
}

fn default_rpc_fallback_endpoints_from_env() -> Vec<RpcEndpointConfig> {
    match std::env::var("SOV_CELESTIA_RPC_SECONDARY_URL") {
        Ok(url) if !url.trim().is_empty() => vec![RpcEndpointConfig {
            url,
            token: std::env::var("SOV_CELESTIA_RPC_SECONDARY_AUTH_TOKEN").ok(),
        }],
        _ => Vec::new(),
    }
}

fn default_grpc_fallback_endpoints_from_env() -> Vec<GrpcEndpointConfig> {
    match std::env::var("SOV_CELESTIA_GRPC_SECONDARY_URL") {
        Ok(url) if !url.trim().is_empty() => vec![GrpcEndpointConfig {
            url,
            token: std::env::var("SOV_CELESTIA_GRPC_SECONDARY_AUTH_TOKEN").ok(),
        }],
        _ => Vec::new(),
    }
}

fn default_fibre_request_timeout_secs() -> NonZero<u64> {
    NonZero::new(30).expect("30 is not zero")
}

fn default_request_timeout_secs() -> NonZero<u64> {
    NonZero::new(20).expect("20 is not zero")
}

fn default_api_request_timeout_secs() -> NonZero<u64> {
    NonZero::new(5).expect("5 is not zero")
}

fn default_backoff_min_delay_ms() -> u64 {
    100
}

fn default_backoff_max_delay_ms() -> u64 {
    12_000
}

fn default_backoff_max_times() -> usize {
    60
}

fn default_backoff_factor() -> f32 {
    2.0
}

impl fmt::Debug for CelestiaFibreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CelestiaFibreConfig")
            .field("rpc_url", &self.rpc_url)
            .field(
                "rpc_auth_token",
                &self.rpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field("rpc_fallback_endpoints", &self.rpc_fallback_endpoints)
            .field(
                "rpc_health_check_interval_secs",
                &self.rpc_health_check_interval_secs,
            )
            .field("rpc_max_head_age_secs", &self.rpc_max_head_age_secs)
            .field("grpc_url", &self.grpc_url)
            .field(
                "grpc_auth_token",
                &self.grpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field("grpc_fallback_endpoints", &self.grpc_fallback_endpoints)
            .field("fibre_grpc_url", &self.fibre_grpc_url)
            .field(
                "fibre_grpc_auth_token",
                &self.fibre_grpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field(
                "signer_private_key",
                &self.signer_private_key.as_ref().map(|_| "REDACTED"),
            )
            .field(
                "fibre_request_timeout_secs",
                &self.fibre_request_timeout_secs,
            )
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("api_request_timeout_secs", &self.api_request_timeout_secs)
            .field("backoff_min_delay_ms", &self.backoff_min_delay_ms)
            .field("backoff_max_delay_ms", &self.backoff_max_delay_ms)
            .field("backoff_max_times", &self.backoff_max_times)
            .field("backoff_factor", &self.backoff_factor)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_rollup_config_parses() {
        #[derive(serde::Deserialize)]
        struct RollupConfig {
            da: CelestiaFibreConfig,
        }
        let example = include_str!(
            "../../../../examples/demo-rollup/configs/celestia_fibre_rollup_config.toml"
        );

        let config: RollupConfig = toml::from_str(example).unwrap();

        assert_eq!(
            config.da.grpc_url,
            "https://grpc.da-proxy-lb.celestia-mocha.com:443"
        );
    }
}
