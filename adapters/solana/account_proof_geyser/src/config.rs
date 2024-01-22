use std::net::SocketAddr;
use std::path::Path;
use std::{fs, io};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub libpath: String,
    pub account_list: Vec<String>,
    pub bind_address: SocketAddr,
}

#[derive(Debug)]
pub enum ConfigError {
    IoError(io::Error),
    ParseError(serde_json::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ConfigError::IoError(e) => write!(f, "I/O error: {}", e),
            ConfigError::ParseError(e) => write!(f, "Parse error: {}", e),
        }
    }
}

impl From<io::Error> for ConfigError {
    fn from(err: io::Error) -> Self {
        ConfigError::IoError(err)
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(err: serde_json::Error) -> Self {
        ConfigError::ParseError(err)
    }
}

impl Config {
    fn load_from_str(config: &str) -> Result<Self, ConfigError> {
        serde_json::from_str(config).map_err(ConfigError::from)
    }

    pub fn load_from_file<P: AsRef<Path>>(file: P) -> Result<Self, ConfigError> {
        let config = fs::read_to_string(file)?;
        Self::load_from_str(&config)
    }
}
