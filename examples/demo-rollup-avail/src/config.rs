use serde::Deserialize;
use sov_stf_runner::RollupConfig;

//TODO - replace with runtime config.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DaServiceConfig {
    pub light_client_url: String,
    pub node_client_url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    pub rollup_config: RollupConfig,
    pub sequencer_da_address: String,
    pub da: DaServiceConfig,
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use sov_stf_runner::{from_toml_path, RpcConfig, StorageConfig, Config as RunnerConfig};
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
        sequencer_da_address = "b4dc7fc57630d2a7be7f358cbefc1e52bd6d0f250d19647cf264ecf2d8764d7b"
        [rollup_config]
        start_height = 2
        [da]
        light_client_url = "http://127.0.0.1:8000"
        node_client_url = "wss://kate.avail.tools:443/ws"
        [rollup_config.runner.storage]
        path = "demo_data"
        [rollup_config.rpc_config]
        bind_host = "127.0.0.1"
        bind_port = 12345
        "#;

        let config_file = create_config_from(config);

        let config: Config = from_toml_path(config_file.path()).unwrap();
        let expected = Config {
            sequencer_da_address: String::from(
                "b4dc7fc57630d2a7be7f358cbefc1e52bd6d0f250d19647cf264ecf2d8764d7b",
            ),
            da: DaServiceConfig {
                light_client_url: "http://127.0.0.1:8000".to_string(),
                node_client_url: "wss://kate.avail.tools:443/ws".into(),
            },
            rollup_config: RollupConfig {
                start_height: 2,
                runner: RunnerConfig {
                    storage: StorageConfig {
                        path: PathBuf::from("demo_data"),
                    },
                },
                rpc_config: RpcConfig {
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 12345,
                },
            },
        };
        assert_eq!(config, expected);
    }
}
