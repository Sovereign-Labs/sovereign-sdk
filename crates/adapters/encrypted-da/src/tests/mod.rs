use sov_encryption::config::{CipherType, EncryptionConfig, KeyClientConfig};
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::MockAddress;

mod fixtures;
mod mock_da_tests;
mod celestia_tests;

/// Helper function to create a test encryption config
pub(crate) fn test_encryption_config() -> EncryptionConfig {
    EncryptionConfig {
        key_client: KeyClientConfig::Static {
            encryption_key: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
            decryption_key: None,
        },
        cipher_type: CipherType::Aes256Gcm,
        key_rotation_interval: None,
    }
}

/// Helper function to create a test encryption config for Celestia tests (different key)
pub(crate) fn celestia_test_encryption_config() -> EncryptionConfig {
    EncryptionConfig {
        key_client: KeyClientConfig::Static {
            encryption_key: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
            decryption_key: None,
        },
        cipher_type: CipherType::Aes256Gcm,
        key_rotation_interval: None,
    }
}

/// Helper function to create a mock DA service for testing
pub(crate) async fn create_mock_da() -> StorableMockDaService {
    let sequencer_address = MockAddress::new([1u8; 32]);
    StorableMockDaService::new_in_memory(sequencer_address, 1).await
}