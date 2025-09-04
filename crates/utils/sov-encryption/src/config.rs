use std::time::Duration;

#[cfg(feature = "unix-client")]
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EncryptionConfig {
    /// Key client configuration
    pub key_client: KeyClientConfig,
    
    /// Encryption cipher type to use
    #[serde(default)]
    pub cipher_type: CipherType,
    
    /// Key rotation interval (in seconds). If None, keys don't rotate
    pub key_rotation_interval: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeyClientConfig {
    /// Static key configuration (no external fetching)
    Static {
        /// Hex-encoded encryption key
        encryption_key: String,
        /// Hex-encoded decryption key (if different from encryption key)
        decryption_key: Option<String>,
    },
    
    /// Unix socket key client
    #[cfg(feature = "unix-client")]
    UnixSocket {
        /// Path to the Unix socket
        socket_path: PathBuf,
        /// Timeout for key server requests (in seconds)
        #[serde(default = "default_key_server_timeout")]
        timeout: u64,
        /// Maximum number of retry attempts
        #[serde(default = "default_max_retries")]
        max_retries: u32,
        /// Retry backoff base delay (in milliseconds)
        #[serde(default = "default_retry_delay_ms")]
        retry_delay_ms: u64,
    },
    
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub enum CipherType {
    #[default]
    #[serde(rename = "aes256-gcm")]
    Aes256Gcm,
}

#[cfg(feature = "unix-client")]
fn default_key_server_timeout() -> u64 {
    30 // 30 seconds
}

#[cfg(feature = "unix-client")]
fn default_max_retries() -> u32 {
    3
}

#[cfg(feature = "unix-client")]
fn default_retry_delay_ms() -> u64 {
    1000 // 1 second
}

impl EncryptionConfig {
    pub fn key_rotation_interval_duration(&self) -> Option<Duration> {
        self.key_rotation_interval.map(Duration::from_secs)
    }
}

impl KeyClientConfig {
    pub fn timeout_duration(&self) -> Option<Duration> {
        match self {
            KeyClientConfig::Static { .. } => None,
            #[cfg(feature = "unix-client")]
            KeyClientConfig::UnixSocket { timeout, .. } => Some(Duration::from_secs(*timeout)),
        }
    }
    
    pub fn retry_delay_duration(&self) -> Option<Duration> {
        match self {
            KeyClientConfig::Static { .. } => None,
            #[cfg(feature = "unix-client")]
            KeyClientConfig::UnixSocket { retry_delay_ms, .. } => Some(Duration::from_millis(*retry_delay_ms)),
        }
    }
    
    pub fn max_retries(&self) -> u32 {
        match self {
            KeyClientConfig::Static { .. } => 0,
            #[cfg(feature = "unix-client")]
            KeyClientConfig::UnixSocket { max_retries, .. } => *max_retries,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyRequest {
    /// The type of key being requested
    pub key_type: KeyType,
    /// Optional key identifier for specific key requests
    pub key_id: Option<String>,
    /// Request timestamp
    pub timestamp: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyResponse {
    /// The encryption key (hex-encoded)
    pub key: String,
    /// Key identifier
    pub key_id: String,
    /// Key expiration timestamp (Unix timestamp)
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum KeyType {
    #[serde(rename = "encryption")]
    Encryption,
    #[serde(rename = "decryption")]
    Decryption,
}