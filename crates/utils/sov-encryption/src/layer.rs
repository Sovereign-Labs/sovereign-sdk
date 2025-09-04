use std::sync::Arc;
use std::fmt;

use async_trait::async_trait;
use tracing::debug;

#[cfg(feature = "aes-encryption")]
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
#[cfg(feature = "aes-encryption")]
use rand::{RngCore, rngs::OsRng};

use crate::{
    config::{CipherType, EncryptionConfig},
    error::EncryptionError,
    key_client::{KeyClient, create_key_client},
};

#[cfg(feature = "aes-encryption")]
const AES_256_KEY_SIZE: usize = 32;
#[cfg(feature = "aes-encryption")]
const AES_GCM_NONCE_SIZE: usize = 12;

#[async_trait]
pub trait EncryptionLayerTrait: Send + Sync {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError>;
    async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError>;
    fn encryption_type(&self) -> &'static str;
}

impl fmt::Debug for dyn EncryptionLayerTrait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EncryptionLayer({})", self.encryption_type())
    }
}

#[cfg(feature = "aes-encryption")]
pub struct AesGcmEncryptionLayer {
    key_client: Arc<Box<dyn KeyClient>>,
}

#[cfg(feature = "aes-encryption")]
impl AesGcmEncryptionLayer {
    pub fn new(config: EncryptionConfig) -> Self {
        let key_client = Arc::new(create_key_client(config.key_client));
        
        Self {
            key_client,
        }
    }

    pub fn new_with_key_client(key_client: Arc<Box<dyn KeyClient>>) -> Self {
        Self {
            key_client,
        }
    }

    fn encrypt_with_key(&self, key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if key.len() != AES_256_KEY_SIZE {
            return Err(EncryptionError::InvalidKeyFormat(
                format!("Expected {} byte key, got {}", AES_256_KEY_SIZE, key.len())
            ));
        }

        let cipher_key = Key::<Aes256Gcm>::from_slice(key);
        let cipher = Aes256Gcm::new(cipher_key);

        // Generate a random nonce
        let mut nonce_bytes = [0u8; AES_GCM_NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        debug!("Encrypting {} bytes with AES-256-GCM", plaintext.len());

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("AES-GCM encryption failed: {e}")))?;

        // Prepend nonce to ciphertext for storage
        let mut result = Vec::with_capacity(AES_GCM_NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        debug!("Successfully encrypted to {} bytes (including nonce)", result.len());
        Ok(result)
    }

    fn decrypt_with_key(&self, key: &[u8], ciphertext_with_nonce: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if key.len() != AES_256_KEY_SIZE {
            return Err(EncryptionError::InvalidKeyFormat(
                format!("Expected {} byte key, got {}", AES_256_KEY_SIZE, key.len())
            ));
        }

        if ciphertext_with_nonce.len() < AES_GCM_NONCE_SIZE {
            return Err(EncryptionError::InvalidCiphertextFormat(
                format!("Ciphertext too short: expected at least {} bytes, got {}", 
                        AES_GCM_NONCE_SIZE, ciphertext_with_nonce.len())
            ));
        }

        let cipher_key = Key::<Aes256Gcm>::from_slice(key);
        let cipher = Aes256Gcm::new(cipher_key);

        // Extract nonce from the beginning
        let (nonce_bytes, ciphertext) = ciphertext_with_nonce.split_at(AES_GCM_NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        debug!("Decrypting {} bytes with AES-256-GCM", ciphertext.len());

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| EncryptionError::DecryptionFailed(format!("AES-GCM decryption failed: {e}")))?;

        debug!("Successfully decrypted to {} bytes", plaintext.len());
        Ok(plaintext)
    }
}

#[cfg(feature = "aes-encryption")]
#[async_trait]
impl EncryptionLayerTrait for AesGcmEncryptionLayer {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        tracing::info!("🔐 ENCRYPT: Starting encryption of {} bytes", plaintext.len());
        debug!("Requesting encryption key");
        let key = self.key_client.get_encryption_key().await?;
        
        // Log key info safely (first/last few bytes)
        if key.len() >= 8 {
            tracing::info!("🔑 Using key: {}...{}", hex::encode(&key[..4]), hex::encode(&key[key.len()-4..]));
        }
        
        let result = self.encrypt_with_key(&key, plaintext)?;
        tracing::info!("🔐 ENCRYPT: Completed, output {} bytes", result.len());
        Ok(result)
    }

    async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        tracing::info!("🔓 DECRYPT: Starting decryption of {} bytes", ciphertext.len());
        debug!("Requesting decryption key");
        // For now, we use the same key for encryption and decryption
        // In a more advanced implementation, you might extract a key ID from the ciphertext
        let key = self.key_client.get_decryption_key(None).await?;
        
        // Log key info safely (first/last few bytes)
        if key.len() >= 8 {
            tracing::info!("🔑 Using key: {}...{}", hex::encode(&key[..4]), hex::encode(&key[key.len()-4..]));
        }
        
        let result = self.decrypt_with_key(&key, ciphertext)?;
        tracing::info!("🔓 DECRYPT: Completed, output {} bytes", result.len());
        Ok(result)
    }

    fn encryption_type(&self) -> &'static str {
        "AES-256-GCM"
    }
}

pub fn create_encryption_layer(config: EncryptionConfig) -> Box<dyn EncryptionLayerTrait> {
    match config.cipher_type {
        #[cfg(feature = "aes-encryption")]
        CipherType::Aes256Gcm => Box::new(AesGcmEncryptionLayer::new(config)),
        #[cfg(not(feature = "aes-encryption"))]
        CipherType::Aes256Gcm => panic!("AES-256-GCM encryption requires the 'aes-encryption' feature to be enabled"),
    }
}

// Convenience type alias and struct for easier usage
pub struct EncryptionLayer {
    inner: Box<dyn EncryptionLayerTrait>,
}

impl EncryptionLayer {
    pub async fn new(config: EncryptionConfig) -> Result<Self, EncryptionError> {
        let inner = create_encryption_layer(config);
        Ok(Self { inner })
    }

    pub async fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Option<String>), EncryptionError> {
        let encrypted = self.inner.encrypt(plaintext).await?;
        // For now, we don't use key IDs, but this supports the interface
        Ok((encrypted, None))
    }

    pub async fn decrypt(&self, ciphertext: &[u8], _key_id: Option<&str>) -> Result<Vec<u8>, EncryptionError> {
        // Key ID is ignored for now since we use the same key for encryption/decryption
        self.inner.decrypt(ciphertext).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::key_client::StaticKeyClient;

    #[cfg(feature = "aes-encryption")]
    // Mock key client for testing
    struct MockKeyClient {
        key: Vec<u8>,
    }

    #[cfg(feature = "aes-encryption")]
    impl MockKeyClient {
        fn new() -> Self {
            // Generate a random 256-bit key for testing
            let mut key = vec![0u8; AES_256_KEY_SIZE];
            OsRng.fill_bytes(&mut key);
            Self { key }
        }
    }

    #[cfg(feature = "aes-encryption")]
    #[async_trait]
    impl KeyClient for MockKeyClient {
        async fn get_encryption_key(&self) -> Result<Vec<u8>, EncryptionError> {
            Ok(self.key.clone())
        }

        async fn get_decryption_key(&self, _key_id: Option<String>) -> Result<Vec<u8>, EncryptionError> {
            Ok(self.key.clone())
        }
    }

    #[cfg(feature = "aes-encryption")]
    #[tokio::test]
    async fn test_encrypt_decrypt_roundtrip() {
        let mock_client = Arc::new(Box::new(MockKeyClient::new()) as Box<dyn KeyClient>);
        let layer = AesGcmEncryptionLayer::new_with_key_client(mock_client);

        let plaintext = b"Hello, world! This is a test message for encryption.";
        
        // Encrypt
        let ciphertext = layer.encrypt(plaintext).await.unwrap();
        assert!(ciphertext.len() > plaintext.len()); // Should be larger due to nonce + auth tag
        assert_ne!(ciphertext, plaintext); // Should be different

        // Decrypt
        let decrypted = layer.decrypt(&ciphertext).await.unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[cfg(feature = "aes-encryption")]
    #[tokio::test]
    async fn test_encrypt_produces_different_ciphertexts() {
        let mock_client = Arc::new(Box::new(MockKeyClient::new()) as Box<dyn KeyClient>);
        let layer = AesGcmEncryptionLayer::new_with_key_client(mock_client);

        let plaintext = b"Same message";
        
        // Encrypt the same message twice
        let ciphertext1 = layer.encrypt(plaintext).await.unwrap();
        let ciphertext2 = layer.encrypt(plaintext).await.unwrap();
        
        // Should produce different ciphertexts due to random nonces
        assert_ne!(ciphertext1, ciphertext2);
        
        // But both should decrypt to the same plaintext
        let decrypted1 = layer.decrypt(&ciphertext1).await.unwrap();
        let decrypted2 = layer.decrypt(&ciphertext2).await.unwrap();
        assert_eq!(decrypted1, plaintext);
        assert_eq!(decrypted2, plaintext);
    }

    #[cfg(feature = "aes-encryption")]
    #[tokio::test]
    async fn test_decrypt_invalid_ciphertext() {
        let mock_client = Arc::new(Box::new(MockKeyClient::new()) as Box<dyn KeyClient>);
        let layer = AesGcmEncryptionLayer::new_with_key_client(mock_client);

        // Test with ciphertext that's too short
        let result = layer.decrypt(b"short").await;
        assert!(matches!(result, Err(EncryptionError::InvalidCiphertextFormat(_))));

        // Test with invalid ciphertext (random bytes)
        let mut invalid_ciphertext = vec![0u8; AES_GCM_NONCE_SIZE + 32];
        OsRng.fill_bytes(&mut invalid_ciphertext);
        
        let result = layer.decrypt(&invalid_ciphertext).await;
        assert!(matches!(result, Err(EncryptionError::DecryptionFailed(_))));
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_encrypt_with_invalid_key_size() {
        let mock_client = Arc::new(Box::new(MockKeyClient::new()) as Box<dyn KeyClient>);
        let layer = AesGcmEncryptionLayer::new_with_key_client(mock_client);

        let invalid_key = vec![0u8; 16]; // Wrong size (should be 32)
        let plaintext = b"test message";

        let result = layer.encrypt_with_key(&invalid_key, plaintext);
        assert!(matches!(result, Err(EncryptionError::InvalidKeyFormat(_))));
    }

    // Test static key client integration
    #[tokio::test]
    async fn test_static_key_client_integration() {
        let hex_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let static_client = Arc::new(Box::new(StaticKeyClient::new(hex_key.to_string(), None)) as Box<dyn KeyClient>);
        
        // This should work without any encryption features
        let enc_key = static_client.get_encryption_key().await.unwrap();
        let dec_key = static_client.get_decryption_key(None).await.unwrap();
        
        assert_eq!(enc_key.len(), 32);
        assert_eq!(enc_key, dec_key);
    }
    
    // Test create_key_client with different config types
    #[test]
    fn test_create_key_client_configs() {
        use crate::config::KeyClientConfig;
        
        let static_config = KeyClientConfig::Static {
            encryption_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            decryption_key: None,
        };
        
        let _client = create_key_client(static_config);
        
        #[cfg(feature = "unix-client")]
        {
            let unix_config = KeyClientConfig::UnixSocket {
                socket_path: "/tmp/test.sock".into(),
                timeout: 30,
                max_retries: 3,
                retry_delay_ms: 1000,
            };
            let _unix_client = create_key_client(unix_config);
        }
        
        #[cfg(feature = "http-client")]
        {
            let http_config = KeyClientConfig::Http {
                base_url: "http://localhost:8080".to_string(),
                timeout: 30,
                max_retries: 3,
                retry_delay_ms: 1000,
                auth_headers: std::collections::HashMap::new(),
            };
            let _http_client = create_key_client(http_config);
        }
    }

    // Comprehensive AES encryption tests
    #[cfg(feature = "aes-encryption")]
    mod aes_comprehensive_tests {
        use super::*;
        use crate::config::{EncryptionConfig, KeyClientConfig, CipherType};

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

        #[tokio::test]
        async fn test_aes_encryption_different_keys() {
            let config1 = EncryptionConfig {
                key_client: KeyClientConfig::Static {
                    encryption_key: "1111111111111111111111111111111111111111111111111111111111111111".to_string(),
                    decryption_key: None,
                },
                cipher_type: CipherType::Aes256Gcm,
                key_rotation_interval: None,
            };

            let config2 = EncryptionConfig {
                key_client: KeyClientConfig::Static {
                    encryption_key: "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
                    decryption_key: None,
                },
                cipher_type: CipherType::Aes256Gcm,
                key_rotation_interval: None,
            };
            
            let layer1 = create_encryption_layer(config1);
            let layer2 = create_encryption_layer(config2);
            
            let test_data = b"test data for different keys";
            
            let encrypted1 = layer1.encrypt(test_data).await.unwrap();
            let encrypted2 = layer2.encrypt(test_data).await.unwrap();
            
            // Different keys should produce different ciphertexts
            assert_ne!(encrypted1, encrypted2);
            
            // Each should decrypt with their own key
            assert_eq!(layer1.decrypt(&encrypted1).await.unwrap(), test_data);
            assert_eq!(layer2.decrypt(&encrypted2).await.unwrap(), test_data);
            
            // But not with each other's key
            assert!(layer1.decrypt(&encrypted2).await.is_err());
            assert!(layer2.decrypt(&encrypted1).await.is_err());
        }

        #[tokio::test]
        async fn test_aes_nonce_randomness() {
            let encryption_config = create_test_encryption_config();
            let layer = create_encryption_layer(encryption_config);
            
            let test_data = b"same message multiple times";
            
            // Encrypt same data multiple times
            let encrypted1 = layer.encrypt(test_data).await.unwrap();
            let encrypted2 = layer.encrypt(test_data).await.unwrap();
            let encrypted3 = layer.encrypt(test_data).await.unwrap();
            
            // Should all be different due to random nonces
            assert_ne!(encrypted1, encrypted2);
            assert_ne!(encrypted2, encrypted3);
            assert_ne!(encrypted1, encrypted3);
            
            // But all should decrypt to same plaintext
            assert_eq!(layer.decrypt(&encrypted1).await.unwrap(), test_data);
            assert_eq!(layer.decrypt(&encrypted2).await.unwrap(), test_data);
            assert_eq!(layer.decrypt(&encrypted3).await.unwrap(), test_data);
        }

        #[tokio::test]
        async fn test_aes_empty_data() {
            let encryption_config = create_test_encryption_config();
            let layer = create_encryption_layer(encryption_config);
            
            let empty_data = b"";
            let encrypted = layer.encrypt(empty_data).await.unwrap();
            let decrypted = layer.decrypt(&encrypted).await.unwrap();
            
            assert_eq!(decrypted, empty_data);
            assert!(!encrypted.is_empty()); // Should still have nonce and auth tag
        }

        #[tokio::test]
        async fn test_aes_large_data() {
            let encryption_config = create_test_encryption_config();
            let layer = create_encryption_layer(encryption_config);
            
            // Test with 1MB of data
            let large_data = vec![42u8; 1024 * 1024];
            let encrypted = layer.encrypt(&large_data).await.unwrap();
            let decrypted = layer.decrypt(&encrypted).await.unwrap();
            
            assert_eq!(decrypted, large_data);
        }

        #[tokio::test]
        async fn test_aes_invalid_ciphertext() {
            let encryption_config = create_test_encryption_config();
            let layer = create_encryption_layer(encryption_config);
            
            // Test with invalid ciphertext (too short)
            let result = layer.decrypt(b"short").await;
            assert!(result.is_err());
            
            // Test with random invalid data
            let invalid_data = vec![42u8; 50];
            let result = layer.decrypt(&invalid_data).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_create_encryption_layer_from_config() {
            let encryption_config = create_test_encryption_config();
            let layer = create_encryption_layer(encryption_config);
            
            let test_data = b"AES encryption test data with sufficient length for testing";
            
            // Test encryption
            let encrypted = layer.encrypt(test_data).await.unwrap();
            assert_ne!(encrypted, test_data);
            assert!(!encrypted.is_empty()); // Should be larger due to nonce + auth tag
            
            // Test decryption
            let decrypted = layer.decrypt(&encrypted).await.unwrap();
            assert_eq!(decrypted, test_data);
        }
    }

    // Test Debug implementation
    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_debug_implementation() {
        let hex_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let static_client = Arc::new(Box::new(StaticKeyClient::new(hex_key.to_string(), None)) as Box<dyn KeyClient>);
        let layer = AesGcmEncryptionLayer::new_with_key_client(static_client);
        
        let boxed_layer: Box<dyn EncryptionLayerTrait> = Box::new(layer);
        let debug_output = format!("{boxed_layer:?}");
        
        assert_eq!(debug_output, "EncryptionLayer(AES-256-GCM)");
    }

    #[cfg(feature = "aes-encryption")]
    #[test]
    fn test_debug_implementation_from_config() {
        use crate::config::{EncryptionConfig, KeyClientConfig, CipherType};
        
        let config = EncryptionConfig {
            key_client: KeyClientConfig::Static {
                encryption_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
                decryption_key: None,
            },
            cipher_type: CipherType::Aes256Gcm,
            key_rotation_interval: None,
        };
        
        let layer = create_encryption_layer(config);
        let debug_output = format!("{layer:?}");
        
        assert_eq!(debug_output, "EncryptionLayer(AES-256-GCM)");
    }
}