use std::time::Duration;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::MockAddress;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::DaService;
use tokio::time::sleep;

use crate::EncryptedDaService;
use super::{test_encryption_config, celestia_test_encryption_config};

/// Test data and fixtures for encryption tests
#[allow(dead_code)]
pub struct EncryptionTestData {
    pub plaintext: Vec<u8>,
    pub key_bytes: [u8; 32],
    pub nonce: [u8; 12],
    pub expected_aes_gcm_ciphertext: Vec<u8>,
}

impl EncryptionTestData {
    /// Create fixed test data for consistent testing
    pub fn new() -> Self {
        let plaintext = b"test_data_for_encryption_verification".to_vec();
        
        // Use the same key as our test encryption config
        let key_hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let key_vec = hex::decode(key_hex).unwrap();
        let key_bytes: [u8; 32] = key_vec.try_into().unwrap();
        
        // Use a fixed nonce for deterministic results
        let nonce = [0u8; 12]; // Fixed nonce for testing
        
        // Create expected ciphertext using AES-GCM directly
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce_obj = Nonce::from_slice(&nonce);
        
        let expected_aes_gcm_ciphertext = cipher.encrypt(nonce_obj, plaintext.as_ref()).unwrap();
        
        Self {
            plaintext,
            key_bytes,
            nonce,
            expected_aes_gcm_ciphertext,
        }
    }
    
    /// Encrypt data using AES-GCM directly (not sovereign-encryption)
    pub fn encrypt_with_aes_gcm(&self, data: &[u8], nonce: &[u8; 12]) -> Vec<u8> {
        let key = Key::<Aes256Gcm>::from_slice(&self.key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce_obj = Nonce::from_slice(nonce);
        
        cipher.encrypt(nonce_obj, data).unwrap()
    }
    
    /// Decrypt data using AES-GCM directly
    pub fn decrypt_with_aes_gcm(&self, ciphertext: &[u8], nonce: &[u8; 12]) -> Vec<u8> {
        let key = Key::<Aes256Gcm>::from_slice(&self.key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce_obj = Nonce::from_slice(nonce);
        
        cipher.decrypt(nonce_obj, ciphertext).unwrap()
    }
}

/// Pre-encrypted block fixture for decryption testing
pub struct MockDaBlockFixture {
    pub known_plaintext: Vec<u8>,
    pub mock_da: StorableMockDaService,
    pub block_height: u64,
}

impl MockDaBlockFixture {
    /// Create a mock DA with a pre-encrypted block containing known plaintext
    pub async fn create() -> Self {
        let known_plaintext = b"known_plaintext_for_decryption_test_fixture".to_vec();
        
        // Create mock DA
        let sequencer_address = MockAddress::new([99u8; 32]);
        let mock_da = StorableMockDaService::new_in_memory(sequencer_address, 1).await;
        
        // Create encrypted DA service to submit the fixture
        let encryption_config = test_encryption_config();
        let encrypted_da = EncryptedDaService::new(mock_da.clone(), encryption_config);
        
        // Submit the known plaintext (it will be encrypted automatically)
        let receipt_rx = encrypted_da.send_transaction(&known_plaintext).await;
        let _receipt = receipt_rx.await
            .expect("Receipt channel should not be closed")
            .expect("Fixture transaction should be submitted successfully");
        
        // Wait for block production - extended for mock DA reliability
        sleep(Duration::from_millis(500)).await;
        
        // Get the block height where our fixture was stored
        let latest_header = mock_da.get_head_block_header().await
            .expect("Should get head block header");
        let block_height = latest_header.height();
        
        println!("✓ Created mock DA block fixture at height {} with known plaintext", block_height);
        
        Self {
            known_plaintext,
            mock_da,
            block_height,
        }
    }
}

/// Celestia-specific test data for encryption tests (uses different key from MockDA)
#[allow(dead_code)]
pub struct CelestiaEncryptionTestData {
    pub plaintext: Vec<u8>,
    pub key_bytes: [u8; 32],
    pub nonce: [u8; 12],
    pub expected_aes_gcm_ciphertext: Vec<u8>,
}

impl CelestiaEncryptionTestData {
    /// Create fixed test data for consistent testing with Celestia key
    pub fn new() -> Self {
        let plaintext = b"celestia_test_data_for_encryption_verification".to_vec();
        
        // Use the same key as our Celestia test encryption config
        let key_hex = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let key_vec = hex::decode(key_hex).unwrap();
        let key_bytes: [u8; 32] = key_vec.try_into().unwrap();
        
        // Use a fixed nonce for deterministic results
        let nonce = [0u8; 12]; // Fixed nonce for testing
        
        // Create expected ciphertext using AES-GCM directly
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce_obj = Nonce::from_slice(&nonce);
        
        let expected_aes_gcm_ciphertext = cipher.encrypt(nonce_obj, plaintext.as_ref()).unwrap();
        
        Self {
            plaintext,
            key_bytes,
            nonce,
            expected_aes_gcm_ciphertext,
        }
    }
    
    /// Encrypt data using AES-GCM directly (not sovereign-encryption)
    pub fn encrypt_with_aes_gcm(&self, data: &[u8], nonce: &[u8; 12]) -> Vec<u8> {
        let key = Key::<Aes256Gcm>::from_slice(&self.key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce_obj = Nonce::from_slice(nonce);
        
        cipher.encrypt(nonce_obj, data).unwrap()
    }
    
    /// Decrypt data using AES-GCM directly
    pub fn decrypt_with_aes_gcm(&self, ciphertext: &[u8], nonce: &[u8; 12]) -> Vec<u8> {
        let key = Key::<Aes256Gcm>::from_slice(&self.key_bytes);
        let cipher = Aes256Gcm::new(key);
        let nonce_obj = Nonce::from_slice(nonce);
        
        cipher.decrypt(nonce_obj, ciphertext).unwrap()
    }
}

/// Celestia block fixture for decryption testing (uses MockDA as Celestia substitute)
pub struct CelestiaBlockFixture {
    pub known_plaintext: Vec<u8>,
    pub mock_celestia: StorableMockDaService, // Using MockDA to simulate Celestia
    pub block_height: u64,
}

impl CelestiaBlockFixture {
    /// Create a mock Celestia DA with a pre-encrypted block containing known plaintext
    pub async fn create() -> Self {
        let known_plaintext = b"celestia_known_plaintext_for_decryption_test".to_vec();
        
        // Create mock DA to simulate Celestia
        let sequencer_address = MockAddress::new([200u8; 32]); // Different address from MockDA tests
        let mock_celestia = StorableMockDaService::new_in_memory(sequencer_address, 1).await;
        
        // Create encrypted DA service to submit the fixture
        let encryption_config = celestia_test_encryption_config();
        let encrypted_celestia = EncryptedDaService::new(mock_celestia.clone(), encryption_config);
        
        // Submit the known plaintext (it will be encrypted automatically)
        let receipt_rx = encrypted_celestia.send_transaction(&known_plaintext).await;
        let _receipt = receipt_rx.await
            .expect("Receipt channel should not be closed")
            .expect("Celestia fixture transaction should be submitted successfully");
        
        // Wait for block production - extended for mock DA reliability
        sleep(Duration::from_millis(500)).await;
        
        // Get the block height where our fixture was stored
        let latest_header = mock_celestia.get_head_block_header().await
            .expect("Should get head block header from mock Celestia");
        let block_height = latest_header.height();
        
        println!("✓ Created Celestia block fixture at height {} with known plaintext", block_height);
        
        Self {
            known_plaintext,
            mock_celestia,
            block_height,
        }
    }
}