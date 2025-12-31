use std::num::NonZero;
use std::path::Path;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_db::config::RollupDbConfig;
use sov_rollup_interface::node::da::DaService;

use crate::sequencer::{SeqConfigExtension, SequencerConfig, SequencerKindConfig};

pub const DEFAULT_CONCURRENT_SYNC_TASKS: u8 = 5;

/// Configuration for StateTransitionRunner.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    /// Polling interval for the DA service to check the sync status (in milliseconds).
    pub da_polling_interval_ms: u64,
    /// How much total time DA service has to provide block, including re-orgs, retries, etc.
    /// Exceeding this timeout will lead to rollup shutdown.
    #[serde(default = "default_da_total_timeout_sec")]
    pub da_total_timeout_secs: u64,
    /// HTTP Server configuration: On this socket REST API and RPC endpoints are going to listen.
    pub http_config: HttpServerConfig,
    /// How many concurrent tasks to prefetch DA block during sync.
    #[serde(default = "default_concurrent_sync_tasks")]
    pub concurrent_sync_tasks: u8,
    /// How many blocks maximum will be stored in memory.
    /// It affects total concurrent requests done do DA service.
    /// It should be larger than `concurrent_sync_tasks`.
    #[serde(default = "default_prefetch_blocks_capacity")]
    pub pre_fetched_blocks_capacity: NonZero<usize>,
    /// Whether to save transaction bodies to the database.
    #[serde(default)]
    pub save_tx_bodies: bool,
}

fn default_da_total_timeout_sec() -> u64 {
    600
}

fn default_concurrent_sync_tasks() -> u8 {
    5
}

fn default_prefetch_blocks_capacity() -> NonZero<usize> {
    NonZero::new(20).unwrap()
}

/// Configuration for HTTP server(s) exposed by the node.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct HttpServerConfig {
    /// Server host.
    pub bind_host: String,
    /// Server port.
    pub bind_port: u16,
    /// The fully qualified public name of the server, in case the rollup node is running behind a proxy.
    /// For instance:
    /// ```toml
    /// public_address = "https://rollup.example.com"
    /// ```
    pub public_address: Option<String>,
    /// Enable or disable CORS policy headers. Enabled by default.
    #[serde(default)]
    pub cors: CorsConfiguration,
}

/// See [`HttpServerConfig::cors`].
///
/// # Default
///
/// Cross-origin resource sharing (CORS) makes local development easier, so it's enabled by default
///
/// In production, one may want to disable it and let your reverse proxy
/// handle CORS instead.
/// Security note:
/// Allowing CORS in production allows making requests to rollup node any website on the Internet.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CorsConfiguration {
    /// Enables CORS with a permissive policy, intended for local development.
    #[default]
    Permissive,
    /// Disables CORS.
    Restrictive,
}

impl HttpServerConfig {
    /// Creates an [`HttpServerConfig`] that requests the operating system to bind to any available port.
    /// Useful for testing as it prevents multiple threads from binding to the same port.
    pub fn localhost_on_free_port() -> Self {
        Self::localhost_on_port(0)
    }

    /// Creates an [`HttpServerConfig`] that listens on the provided port using sensible defaults
    /// for local testing.
    pub fn localhost_on_port(port: u16) -> Self {
        HttpServerConfig {
            bind_host: "127.0.0.1".to_string(),
            bind_port: port,
            public_address: None,
            cors: CorsConfiguration::Permissive,
        }
    }

    /// Creates an [`HttpServerConfig`] that listens on provided host and port,
    /// using sensible default for local testing
    pub fn on_host_port(host: impl Into<String>, port: u16) -> Self {
        HttpServerConfig {
            bind_host: host.into(),
            bind_port: port,
            public_address: None,
            cors: CorsConfiguration::Permissive,
        }
    }
}

/// Prover service configuration.
#[derive(Debug, Clone, Deserialize, Serialize, Copy, JsonSchema)]
pub struct ProofManagerConfig<Address> {
    /// The "distance" measured in the number of blocks between two consecutive aggregated proofs.
    pub aggregated_proof_block_jump: NonZero<usize>,
    /// The prover receives rewards to this address.
    pub prover_address: Address,
    /// A number of state transition info entries are allowed to be stored in the database.
    /// When the number is exceeded, older entries are removed.
    pub max_number_of_transitions_in_db: NonZero<u64>,
    /// A number of state transition info entries are allowed to be kept in memory.
    /// If the number is exceeded, rollup execution will be blocked until provers cathes up.
    pub max_number_of_transitions_in_memory: NonZero<u64>,
}

/// Rollup Configuration
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(
    bound = "Address: JsonSchema, Da: DaService, M: JsonSchema",
    rename = "RollupConfig"
)]
#[serde(deny_unknown_fields)]
pub struct RollupConfig<Address: Copy, Da: DaService, M> {
    /// Currently rollup config runner only supports storage path parameter
    pub storage: RollupDbConfig,
    /// Runner own configuration.
    pub runner: RunnerConfig,
    /// Data Availability service configuration.
    pub da: Da::Config,
    /// Proof manager configuration.
    pub proof_manager: ProofManagerConfig<Address>,
    /// Sequencer (and batch builder) configuration.
    pub sequencer: SequencerConfig<Address, SequencerKindConfig<Address>>,
    /// Monitoring configuration.
    pub monitoring: M,
}

impl<Address: Copy, Da: DaService, M> RollupConfig<Address, Da, M> {
    pub fn extension_or_panic(&self) -> SeqConfigExtension {
        *self.sequencer
            .extension
            .as_ref()
            .expect("Sequencer config extension is missing. Verify the [sequencer.extension] section in the rollup configuration.")
    }
}

/// Reads toml file as a specific type.
pub fn from_toml_path<P: AsRef<Path>, R: DeserializeOwned>(path: P) -> anyhow::Result<R> {
    let contents = std::fs::read_to_string(&path)?;

    tracing::info!(
        path = path.as_ref().to_string_lossy().to_string(),
        size_in_bytes = contents.len(),
        line_count = contents.lines().count(),
        "Parsing rollup configuration file"
    );

    Ok(toml::from_str(&contents)?)
}

#[cfg(test)]
mod tests {
    use super::RollupConfig;
    use sov_metrics::MonitoringConfig;
    use sov_mock_da::MockDaService;
    use sov_modules_api::Address;

    #[test]
    fn test_correct_config() {
        let config_s = r#"
            [da]
            connection_string = "sqlite:///tmp/mockda.sqlite?mode=rwc"
            sender_address = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
            [da.block_producing.periodic]
            block_time_ms = 1_000
            [storage]
            path = "/tmp"
            [runner]
            da_polling_interval_ms = 10000
            concurrent_sync_tasks = 18
            [runner.http_config]
            bind_host = "127.0.0.1"
            bind_port = 12346
            public_address = "https://rollup.sovereign.xyz"
            cors = "restrictive"
            [monitoring]
            telegraf_address = "udp://192.168.4.5:8543"
            max_datagram_size = 1024
            max_pending_metrics = 2560
            [proof_manager]
            aggregated_proof_block_jump = 22
            prover_address = "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
            max_number_of_transitions_in_db = 1025
            max_number_of_transitions_in_memory = 768
            [sequencer]
            blob_processing_timeout_secs = 60
            max_batch_size_bytes = 1048576
            max_concurrent_blobs = 16
            max_allowed_node_distance_behind = 5
            rollup_address = "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
            [sequencer.standard]
        "#;

        let config =
            toml::from_str::<RollupConfig<Address, MockDaService, MonitoringConfig>>(config_s)
                .unwrap();
        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn test_correct_config_with_postgres() {
        let config_s = r#"
            [da]
            connection_string = "sqlite:///tmp/mockda.sqlite?mode=rwc"
            sender_address = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
            [da.block_producing.periodic]
            block_time_ms = 1_000
            [storage]
            path = "/tmp"
            [runner]
            da_polling_interval_ms = 10000
            concurrent_sync_tasks = 18
            [runner.http_config]
            bind_host = "127.0.0.1"
            bind_port = 12346
            public_address = "https://rollup.sovereign.xyz"
            cors = "restrictive"
            [monitoring]
            telegraf_address = "udp://192.168.4.5:8543"
            max_datagram_size = 1024
            max_pending_metrics = 2560
            [proof_manager]
            aggregated_proof_block_jump = 22
            prover_address = "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
            max_number_of_transitions_in_db = 1025
            max_number_of_transitions_in_memory = 768
            [sequencer]
            blob_processing_timeout_secs = 60
            max_batch_size_bytes = 1048576
            max_concurrent_blobs = 16
            max_allowed_node_distance_behind = 5
            rollup_address = "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
            [sequencer.preferred]
            disable_state_root_consistency_checks = true
            recovery_strategy = "TryToSave"
            batch_execution_time_limit_millis = 2000 
            num_cache_warmup_workers = 0
            ideal_lag_behind_finalized_slot = 3
            is_replica = false
            [sequencer.preferred.postgres_config]
            postgres_connection_string = "postgresql://postgres:pass@localhost:5432/db"
            node_id = "node_1"
            time_till_leader_update_allowed_ms = 1000
        "#;

        let config =
            toml::from_str::<RollupConfig<Address, MockDaService, MonitoringConfig>>(config_s)
                .unwrap();

        insta::assert_json_snapshot!(config);
    }

    #[test]
    fn test_correct_config_with_rate_limiter() {
        let config_s = r#"
            [da]
            connection_string = "sqlite:///tmp/mockda.sqlite?mode=rwc"
            sender_address = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f"
            [da.block_producing.periodic]
            block_time_ms = 1_000
            [storage]
            path = "/tmp"
            [runner]
            da_polling_interval_ms = 10000
            concurrent_sync_tasks = 18
            [runner.http_config]
            bind_host = "127.0.0.1"
            bind_port = 12346
            public_address = "https://rollup.sovereign.xyz"
            cors = "restrictive"
            [monitoring]
            telegraf_address = "udp://192.168.4.5:8543"
            max_datagram_size = 1024
            max_pending_metrics = 2560
            [proof_manager]
            aggregated_proof_block_jump = 22
            prover_address = "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
            max_number_of_transitions_in_db = 1025
            max_number_of_transitions_in_memory = 768
            [sequencer]
            blob_processing_timeout_secs = 60
            max_batch_size_bytes = 1048576
            max_concurrent_blobs = 16
            max_allowed_node_distance_behind = 5
            rollup_address = "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf"
            [sequencer.preferred]
            disable_state_root_consistency_checks = true
            recovery_strategy = "TryToSave"
            batch_execution_time_limit_millis = 2000 
            num_cache_warmup_workers = 0
            ideal_lag_behind_finalized_slot = 3
            is_replica = false
            [sequencer.preferred.postgres_config]
            postgres_connection_string = "postgresql://postgres:pass@localhost:5432/db"
            node_id = "node_1"
            time_till_leader_update_allowed_ms = 1000
            [sequencer.preferred.rate_limiter]
            max_nb_of_concurrent_users_in_rate_limiter = 100000
            max_requests_per_second = 1000000
            address_custom_limits = [["sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf", { resources_per_bucket = 10, refill_rate = 2}]]
            ip_custom_limits = [
                ["157.180.14.244", { resources_per_bucket = 10, refill_rate = 2}],
                ["157.180.34.249", { resources_per_bucket = 8, refill_rate = 2}]
            ]
            [sequencer.preferred.rate_limiter.default_limits]
            resources_per_bucket = 5
            refill_rate = 2
        "#;

        let config =
            toml::from_str::<RollupConfig<Address, MockDaService, MonitoringConfig>>(config_s)
                .unwrap();

        insta::assert_json_snapshot!(config);
    }
}
