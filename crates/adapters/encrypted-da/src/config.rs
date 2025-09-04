use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_encryption::EncryptionConfig;

/// Backwards-compatible DA configuration that can optionally include encryption
/// 
/// This allows existing DA configurations to work unchanged, while enabling
/// encryption by simply adding an `encryption` section to the TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EncryptedDaConfig<T>
where
    T: JsonSchema,
{
    /// The underlying DA service configuration (flattened into the same level)
    #[serde(flatten)]
    pub inner_config: T,
    
    /// Optional encryption configuration - if present, enables encryption
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionConfig>,
}

impl<T> EncryptedDaConfig<T>
where
    T: JsonSchema,
{
    /// Create a new configuration without encryption (backwards compatible)
    pub fn new_unencrypted(inner_config: T) -> Self {
        Self {
            inner_config,
            encryption: None,
        }
    }
    
    /// Create a new configuration with encryption
    pub fn new_encrypted(inner_config: T, encryption: EncryptionConfig) -> Self {
        Self {
            inner_config,
            encryption: Some(encryption),
        }
    }
    
    /// Check if encryption is enabled
    pub fn is_encrypted(&self) -> bool {
        self.encryption.is_some()
    }
    
    /// Get the encryption config if present
    pub fn encryption_config(&self) -> Option<&EncryptionConfig> {
        self.encryption.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sov_encryption::{CipherType, KeyClientConfig};
    use serde::{Serialize, Deserialize};
    use schemars::JsonSchema;
    
    #[cfg(feature = "mock-da")]
    use sov_mock_da;

    fn create_test_encryption_config() -> EncryptionConfig {
        EncryptionConfig {
            key_client: KeyClientConfig::Static {
                encryption_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
                decryption_key: None,
            },
            cipher_type: CipherType::Aes256Gcm,
            key_rotation_interval: None,
        }
    }

    #[test]
    fn test_encryption_config_creation() {
        let config = create_test_encryption_config();
        assert_eq!(config.cipher_type, CipherType::Aes256Gcm);
        assert!(config.key_rotation_interval.is_none());
        
        match config.key_client {
            KeyClientConfig::Static { encryption_key, decryption_key } => {
                assert_eq!(encryption_key.len(), 64); // 32 bytes as hex = 64 chars
                assert!(decryption_key.is_none());
            }
            #[cfg(feature = "unix-client")]
            _ => panic!("Expected static key client config"),
        }
    }

    #[test]  
    fn test_encrypted_da_config_creation() {
        let encryption_config = create_test_encryption_config();
        let da_config = EncryptedDaConfig::new_encrypted(42u32, encryption_config.clone());
        
        assert_eq!(da_config.inner_config, 42u32);
        assert_eq!(da_config.encryption, Some(encryption_config));
    }

    #[test]
    fn test_unencrypted_da_config_creation() {
        let da_config = EncryptedDaConfig::new_unencrypted(42u32);
        
        assert_eq!(da_config.inner_config, 42u32);
        assert_eq!(da_config.encryption, None);
        assert!(!da_config.is_encrypted());
        assert!(da_config.encryption_config().is_none());
    }

    #[test]
    fn test_is_encrypted() {
        let unencrypted = EncryptedDaConfig::new_unencrypted("test");
        let encrypted = EncryptedDaConfig::new_encrypted("test", create_test_encryption_config());
        
        assert!(!unencrypted.is_encrypted());
        assert!(encrypted.is_encrypted());
    }

    #[test]
    fn test_encryption_config_with_key_rotation() {
        let mut config = create_test_encryption_config();
        config.key_rotation_interval = Some(3600); // 1 hour
        
        assert_eq!(config.key_rotation_interval, Some(3600));
        assert_eq!(config.key_rotation_interval_duration(), Some(std::time::Duration::from_secs(3600)));
    }

    #[test]
    fn test_key_client_config_timeout_methods() {
        let static_config = KeyClientConfig::Static {
            encryption_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            decryption_key: None,
        };
        
        assert_eq!(static_config.timeout_duration(), None);
        assert_eq!(static_config.retry_delay_duration(), None);
        assert_eq!(static_config.max_retries(), 0);
    }

    #[cfg(feature = "unix-client")]
    #[test]
    fn test_unix_socket_config_methods() {
        use std::path::Path;
        
        let unix_config = KeyClientConfig::UnixSocket {
            socket_path: Path::new("/tmp/test.sock").to_path_buf(),
            timeout: 30,
            max_retries: 3,
            retry_delay_ms: 1000,
        };
        
        assert_eq!(unix_config.timeout_duration(), Some(std::time::Duration::from_secs(30)));
        assert_eq!(unix_config.retry_delay_duration(), Some(std::time::Duration::from_millis(1000)));
        assert_eq!(unix_config.max_retries(), 3);
    }

    // Test configuration for TOML fixtures
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct TestDaConfig {
        name: String,
        value: u32,
        enabled: bool,
    }

    // TOML fixture tests
    #[test]
    fn test_toml_unencrypted_config() {
        let config_toml = r#"
            name = "test-da"
            value = 42
            enabled = true
        "#;
        
        let config: EncryptedDaConfig<TestDaConfig> = toml::from_str(config_toml).unwrap();
        
        // Should be unencrypted
        assert!(!config.is_encrypted());
        assert!(config.encryption_config().is_none());
        
        // Verify inner config was parsed correctly
        assert_eq!(config.inner_config.name, "test-da");
        assert_eq!(config.inner_config.value, 42);
        assert!(config.inner_config.enabled);
    }

    #[test]
    fn test_toml_encrypted_config() {
        let config_toml = r#"
            name = "encrypted-da"
            value = 123
            enabled = false
            
            [encryption]
            cipher_type = "aes256-gcm"
            [encryption.key_client]
            type = "static"
            encryption_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        "#;
        
        let config: EncryptedDaConfig<TestDaConfig> = toml::from_str(config_toml).unwrap();
        
        // Should be encrypted
        assert!(config.is_encrypted());
        let encryption_config = config.encryption_config().unwrap();
        
        // Verify encryption settings
        assert_eq!(encryption_config.cipher_type, CipherType::Aes256Gcm);
        assert!(encryption_config.key_rotation_interval.is_none());
        
        match &encryption_config.key_client {
            KeyClientConfig::Static { encryption_key, decryption_key } => {
                assert_eq!(encryption_key, "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
                assert!(decryption_key.is_none());
            },
            #[cfg(feature = "unix-client")]
            _ => panic!("Expected static key client config"),
        }
        
        // Verify inner config was parsed correctly
        assert_eq!(config.inner_config.name, "encrypted-da");
        assert_eq!(config.inner_config.value, 123);
        assert!(!config.inner_config.enabled);
    }

    #[test]
    fn test_toml_encrypted_with_different_keys() {
        let config_toml = r#"
            name = "multi-key-da"
            value = 999
            enabled = true
            
            [encryption]
            cipher_type = "aes256-gcm"
            key_rotation_interval = 7200
            [encryption.key_client]
            type = "static"
            encryption_key = "1111111111111111111111111111111111111111111111111111111111111111"
            decryption_key = "2222222222222222222222222222222222222222222222222222222222222222"
        "#;
        
        let config: EncryptedDaConfig<TestDaConfig> = toml::from_str(config_toml).unwrap();
        
        // Should be encrypted
        assert!(config.is_encrypted());
        let encryption_config = config.encryption_config().unwrap();
        
        // Verify encryption settings with key rotation
        assert_eq!(encryption_config.cipher_type, CipherType::Aes256Gcm);
        assert_eq!(encryption_config.key_rotation_interval, Some(7200));
        assert_eq!(encryption_config.key_rotation_interval_duration(), Some(std::time::Duration::from_secs(7200)));
        
        // Verify different keys
        match &encryption_config.key_client {
            KeyClientConfig::Static { encryption_key, decryption_key } => {
                assert_eq!(encryption_key, "1111111111111111111111111111111111111111111111111111111111111111");
                assert_eq!(decryption_key, &Some("2222222222222222222222222222222222222222222222222222222222222222".to_string()));
            },
            #[cfg(feature = "unix-client")]
            _ => panic!("Expected static key client config"),
        }
        
        // Verify inner config
        assert_eq!(config.inner_config.name, "multi-key-da");
        assert_eq!(config.inner_config.value, 999);
        assert!(config.inner_config.enabled);
    }


    #[test]
    fn test_toml_roundtrip_serialization() {
        // Create a config programmatically
        let original_config = EncryptedDaConfig::new_encrypted(
            TestDaConfig {
                name: "roundtrip-test".to_string(),
                value: 456,
                enabled: false,
            },
            create_test_encryption_config()
        );
        
        // Serialize to TOML
        let toml_string = toml::to_string(&original_config).unwrap();
        
        // Deserialize back
        let parsed_config: EncryptedDaConfig<TestDaConfig> = toml::from_str(&toml_string).unwrap();
        
        // Should be identical
        assert_eq!(original_config.inner_config.name, parsed_config.inner_config.name);
        assert_eq!(original_config.inner_config.value, parsed_config.inner_config.value);
        assert_eq!(original_config.inner_config.enabled, parsed_config.inner_config.enabled);
        assert_eq!(original_config.is_encrypted(), parsed_config.is_encrypted());
        
        if let (Some(orig_enc), Some(parsed_enc)) = (original_config.encryption_config(), parsed_config.encryption_config()) {
            assert_eq!(orig_enc.cipher_type, parsed_enc.cipher_type);
            assert_eq!(orig_enc.key_rotation_interval, parsed_enc.key_rotation_interval);
        }
    }

    #[test]
    fn test_toml_error_handling() {
        // Invalid TOML should fail gracefully
        let invalid_toml = r#"
            name = "test"
            value = "not_a_number"  # Should be u32
            enabled = true
        "#;
        
        let result: Result<EncryptedDaConfig<TestDaConfig>, _> = toml::from_str(invalid_toml);
        assert!(result.is_err());
        
        // Invalid encryption configuration should fail
        let invalid_encryption_toml = r#"
            name = "test"
            value = 42
            enabled = true
            
            [encryption]
            cipher_type = "invalid_cipher_type"
            [encryption.key_client]
            type = "static"
            encryption_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        "#;
        
        let result: Result<EncryptedDaConfig<TestDaConfig>, _> = toml::from_str(invalid_encryption_toml);
        assert!(result.is_err());
        
        // Missing type field in key_client should fail
        let missing_type_toml = r#"
            name = "test"
            value = 42
            enabled = true
            
            [encryption]
            cipher_type = "aes256-gcm"
            [encryption.key_client]
            encryption_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        "#;
        
        let result: Result<EncryptedDaConfig<TestDaConfig>, _> = toml::from_str(missing_type_toml);
        assert!(result.is_err());
    }
}