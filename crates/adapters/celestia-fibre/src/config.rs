use std::fmt;
use std::num::NonZero;

use schemars::JsonSchema;
use sov_celestia_common::config::{
    backoff_policy, default_api_request_timeout_secs,
    default_background_stat_polling_interval_secs, default_factor, default_grpc_auth_token,
    default_grpc_fallback_endpoints_from_env, default_grpc_url, default_max_delay_ms,
    default_max_times, default_min_delay_ms, default_request_timeout_seconds,
    default_rpc_auth_token, default_rpc_fallback_endpoints_from_env, default_rpc_url_from_env,
    default_safe_lead_time_ms, default_signer_private_key, default_tx_priority,
    default_tx_status_polling_millis, GrpcEndpointConfig, RpcEndpointConfig, TxPriority,
    VerifyOnFetchMode,
};

fn default_download_timeout_secs() -> NonZero<u64> {
    NonZero::new(120).unwrap()
}

#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CelestiaFibreConfig {
    #[serde(default = "default_rpc_url_from_env")]
    pub rpc_url: String,
    #[serde(default = "default_rpc_auth_token")]
    pub rpc_auth_token: Option<String>,
    #[serde(default = "default_rpc_fallback_endpoints_from_env")]
    pub rpc_fallback_endpoints: Vec<RpcEndpointConfig>,
    #[serde(default)]
    pub rpc_health_check_interval_secs: Option<NonZero<u64>>,
    #[serde(default)]
    pub rpc_max_head_age_secs: Option<NonZero<u64>>,
    #[serde(default = "default_grpc_url")]
    pub grpc_url: Option<String>,
    #[serde(default = "default_grpc_auth_token")]
    pub grpc_auth_token: Option<String>,
    #[serde(default = "default_grpc_fallback_endpoints_from_env")]
    pub grpc_fallback_endpoints: Vec<GrpcEndpointConfig>,
    #[serde(default = "default_signer_private_key")]
    pub signer_private_key: Option<String>,
    #[serde(default)]
    pub fibre_grpc_url: Option<String>,
    #[serde(default)]
    pub fibre_grpc_auth_token: Option<String>,
    #[serde(default = "default_download_timeout_secs")]
    pub download_timeout_secs: NonZero<u64>,
    #[serde(default = "default_request_timeout_seconds")]
    pub request_timeout_secs: NonZero<u64>,
    #[serde(default = "default_api_request_timeout_secs")]
    pub api_request_timeout_secs: NonZero<u64>,
    #[serde(default = "default_safe_lead_time_ms")]
    pub safe_lead_time_ms: u64,
    #[serde(default = "default_tx_priority")]
    pub tx_priority: TxPriority,
    #[serde(default = "default_min_delay_ms")]
    pub backoff_min_delay_ms: u64,
    #[serde(default = "default_max_delay_ms")]
    pub backoff_max_delay_ms: u64,
    #[serde(default = "default_max_times")]
    pub backoff_max_times: usize,
    #[serde(default = "default_factor")]
    pub backoff_factor: f32,
    #[serde(default = "default_tx_status_polling_millis")]
    pub tx_status_polling_millis: u64,
    #[serde(default = "default_background_stat_polling_interval_secs")]
    pub background_stat_polling_interval_secs: u64,
    #[serde(default)]
    pub verify_on_fetch: VerifyOnFetchMode,
}

impl CelestiaFibreConfig {
    pub fn fibre_grpc_endpoint(&self) -> Option<(&str, Option<&str>)> {
        let url = self
            .fibre_grpc_url
            .as_deref()
            .or(self.grpc_url.as_deref())?;
        let token = self
            .fibre_grpc_auth_token
            .as_deref()
            .or(self.grpc_auth_token.as_deref());
        Some((url, token))
    }

    pub fn backoff(&self) -> backon::ExponentialBuilder {
        backoff_policy(
            self.backoff_min_delay_ms,
            self.backoff_max_delay_ms,
            self.backoff_max_times,
            self.backoff_factor,
        )
    }

    pub fn signing_key(&self) -> anyhow::Result<Option<celestia_client::tx::SigningKey>> {
        let Some(key_hex) = &self.signer_private_key else {
            return Ok(None);
        };
        let bytes =
            hex::decode(key_hex).map_err(|error| anyhow::anyhow!("signer_private_key: {error}"))?;
        celestia_client::tx::SigningKey::from_slice(&bytes)
            .map(Some)
            .map_err(|error| anyhow::anyhow!("signer_private_key: {error}"))
    }
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
            .field("fibre_grpc_url", &self.fibre_grpc_url)
            .field(
                "fibre_grpc_auth_token",
                &self.fibre_grpc_auth_token.as_ref().map(|_| "REDACTED"),
            )
            .field("download_timeout_secs", &self.download_timeout_secs)
            .field("verify_on_fetch", &self.verify_on_fetch)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_debug_redact_secrets() {
        let config: CelestiaFibreConfig = serde_json::from_str(
            r#"{"rpc_url":"ws://localhost:26658","signer_private_key":"abcdabcd","rpc_auth_token":"secret-token"}"#,
        )
        .unwrap();
        assert_eq!(config.download_timeout_secs.get(), 120);
        assert_eq!(config.request_timeout_secs.get(), 20);
        let rendered = format!("{config:?}");
        assert!(rendered.contains("REDACTED"));
        assert!(!rendered.contains("abcdabcd"));
        assert!(!rendered.contains("secret-token"));
    }

    #[test]
    fn missing_signer_is_read_only() {
        let config: CelestiaFibreConfig =
            serde_json::from_str(r#"{"rpc_url":"ws://localhost:26658"}"#).unwrap();
        assert!(config.signing_key().unwrap().is_none());
    }
}
