use std::fs::File;
use std::io::Read;
use std::path::Path;

use celestia::da_service::DaServiceConfig;
use serde::de::DeserializeOwned;
use serde::Deserialize;
pub use sov_state::config::Config as StorageConfig;

/// RPC configuration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RpcConfig {
    /// RPC host.
    pub bind_host: String,
    /// RPC port.
    pub bind_port: u16,
}

/// Rollup Configuration
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    /// DA start height.
    pub start_height: u64,
    /// DA configuration.
    pub da: DaServiceConfig,
    /// Runner configuration.
    pub storage: StorageConfig,
    /// RPC configuration.
    pub rpc_config: RpcConfig,
}

///TODO
pub fn from_toml_path<P: AsRef<Path>, R: DeserializeOwned>(path: P) -> anyhow::Result<R> {
    let mut contents = String::new();
    {
        let mut file = File::open(path)?;
        file.read_to_string(&mut contents)?;
    }

    let result: R = toml::from_str(&contents)?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

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
            [storage]
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
            storage: StorageConfig {
                path: PathBuf::from("/tmp"),
            },
            rpc_config: RpcConfig {
                bind_host: "127.0.0.1".to_string(),
                bind_port: 12345,
            },
        };
        assert_eq!(config, expected);
    }
}
