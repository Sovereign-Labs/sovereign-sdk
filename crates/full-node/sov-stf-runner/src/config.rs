use full_node_configs::runner::RollupConfig as RollupConfigBase;
pub use full_node_configs::runner::{
    from_toml_path, CorsConfiguration, HttpServerConfig, ProofManagerConfig, RunnerConfig,
};
pub use sov_metrics::{MonitoringConfig, TelegrafSocketConfig};

/// With sov-metrics
pub type RollupConfig<Address, Da> = RollupConfigBase<Address, Da, MonitoringConfig>;

#[cfg(test)]
mod tests {
    use sov_mock_da::MockDaService;
    use sov_modules_api::Address;

    use super::RollupConfig;

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
            genesis_height = 31337
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

        let config = toml::from_str::<RollupConfig<Address, MockDaService>>(config_s).unwrap();
        insta::assert_json_snapshot!(config);
    }
}
