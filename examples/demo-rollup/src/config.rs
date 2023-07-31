use demo_stf::runner_config::Config as RunnerConfig;
use jupiter::da_service::DaServiceConfig;
use serde::Deserialize;

/// Struct specifying a configuration for rpc.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RpcConfig {
    /// The host to bind to as a [`String`]
    pub bind_host: String,
    /// The port to bind to as an [`u16`]
    pub bind_port: u16,
}

/// The rollup configuration used for the demo.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    /// The height of the DA layer to start processing the transactions as an [`u64`].
    pub start_height: u64,
    /// The configuration of the DA layer as a [`DaSeviceConfig`] object.
    pub da: DaServiceConfig,
    /// The configuration of the app runner as a [`RunnerConfig`] object.
    pub runner: RunnerConfig,
    /// The rpc configuration as a [`RpcConfig`] object.
    pub rpc_config: RpcConfig,
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use demo_stf::runner_config::{from_toml_path, StorageConfig};
    use tempfile::NamedTempFile;

    use super::*;

    fn create_config_from(content: &str) -> NamedTempFile {
        let mut config_file = NamedTempFile::new().unwrap();
        config_file.write_all(content.as_bytes()).unwrap();
        config_file
    }

    #[test]
    fn test_correct_config() {
        let config = r#"
            start_height = 31337
            [da]
            celestia_rpc_auth_token = "SECRET_RPC_TOKEN"
            celestia_rpc_address = "http://localhost:11111/"
            max_celestia_response_body_size = 980
            [runner.storage]
            path = "/tmp"
            [rpc_config]
            bind_host = "127.0.0.1"
            bind_port = 12345
        "#;

        let config_file = create_config_from(config);

        let config: RollupConfig = from_toml_path(config_file.path()).unwrap();
        let expected = RollupConfig {
            start_height: 31337,
            da: DaServiceConfig {
                celestia_rpc_auth_token: "SECRET_RPC_TOKEN".to_string(),
                celestia_rpc_address: "http://localhost:11111/".into(),
                max_celestia_response_body_size: 980,
                celestia_rpc_timeout_seconds: 60,
            },
            runner: RunnerConfig {
                storage: StorageConfig {
                    path: PathBuf::from("/tmp"),
                },
            },
            rpc_config: RpcConfig {
                bind_host: "127.0.0.1".to_string(),
                bind_port: 12345,
            },
        };
        assert_eq!(config, expected);
    }
}
