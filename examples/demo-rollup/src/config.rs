use demo_stf::runner_config::Config as RunnerConfig;
use jupiter::da_service::DaServiceConfig;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RpcConfig {
    pub bind_host: String,
    pub bind_port: u16,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    pub start_height: u64,
    pub da: DaServiceConfig,
    pub runner: RunnerConfig,
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
