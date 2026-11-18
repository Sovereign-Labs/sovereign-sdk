use std::fmt;
use std::num::NonZero;
use std::time::Duration;

/// Runtime configuration for [`crate::CelestiaFibreDaService`].
///
/// Timeout and backoff parameters have the same names, meaning and defaults as in
/// `sov_celestia_adapter::CelestiaConfig`.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CelestiaFibreConfig {
    /// Celestia bridge node RPC, for example `ws://localhost:26658`.
    /// Serves headers and the v2 shares that reference Fibre blobs.
    pub rpc_url: String,
    /// JWT token for the RPC server.
    pub rpc_auth_token: Option<String>,
    /// Celestia consensus node gRPC, for example `http://localhost:9090`.
    /// Fibre needs it for both downloads (validator set) and submissions.
    pub grpc_url: String,
    /// Optional gRPC authentication token, sent as `x-token` metadata.
    pub grpc_auth_token: Option<String>,
    /// Hex-encoded secp256k1 private key. Without it the service is read-only.
    pub signer_private_key: Option<String>,
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

    pub(crate) fn backoff_policy(&self) -> backon::ExponentialBuilder {
        backon::ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(self.backoff_min_delay_ms))
            .with_max_times(self.backoff_max_times)
            .with_max_delay(Duration::from_millis(self.backoff_max_delay_ms))
            .with_factor(self.backoff_factor)
    }
}

// Defaults below match `sov_celestia_adapter::CelestiaConfig`; keep them in sync.

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
            .field("grpc_url", &self.grpc_url)
            .field(
                "grpc_auth_token",
                &self.grpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field(
                "signer_private_key",
                &self.signer_private_key.as_ref().map(|_| "REDACTED"),
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
