use std::sync::{Arc, RwLock};
use std::fmt;
use std::path::Path;

use tokio::task::JoinHandle;
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};

#[cfg(feature = "aes-encryption")]
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
#[cfg(feature = "aes-encryption")]
use rand::{RngCore, rngs::OsRng};

use crate::{
    config::EncryptionConfig,
    error::EncryptionError,
};

#[cfg(feature = "aes-encryption")]
const AES_256_KEY_SIZE: usize = 32;
#[cfg(feature = "aes-encryption")]
const AES_GCM_NONCE_SIZE: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionKey {
    pub id: String,
    pub material: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyUpdate {
    NewKey(EncryptionKey),
    RotateKey { old_id: String, new_key: EncryptionKey },
    RevokeKey(String),
}

#[derive(Clone)]
pub struct KeyCache {
    current_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl KeyCache {
    pub fn new() -> Self {
        Self {
            current_key: Arc::new(RwLock::new(None)),
        }
    }
    
    pub fn get_current_key(&self) -> Option<EncryptionKey> {
        self.current_key.read().unwrap().clone()
    }
    
    pub fn update_current_key(&self, key: EncryptionKey) {
        *self.current_key.write().unwrap() = Some(key);
    }
    
    pub fn rotate_key(&self, _old_id: String, new_key: EncryptionKey) {
        // Could keep old key for decryption of old data
        self.update_current_key(new_key);
    }
    
    pub fn revoke_key(&self, key_id: String) {
        let mut current = self.current_key.write().unwrap();
        if let Some(ref key) = *current {
            if key.id == key_id {
                *current = None;
                warn!("Current key revoked, encryption will fail until new key received");
            }
        }
    }
}

impl Default for KeyCache {
    fn default() -> Self {
        Self::new()
    }
}

pub trait EncryptionLayerTrait: Send + Sync {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError>;
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError>;
    fn encryption_type(&self) -> &'static str;
}

impl fmt::Debug for dyn EncryptionLayerTrait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EncryptionLayer({})", self.encryption_type())
    }
}



// Convenience type alias and struct for easier usage
pub struct EncryptionLayer {
    key_cache: KeyCache,
    _key_listener_handle: Option<JoinHandle<()>>,
}

impl EncryptionLayer {
    pub async fn new(config: EncryptionConfig) -> Result<Self, EncryptionError> {
        let key_cache = KeyCache::new();
        
        // Handle different key client configurations
        let key_listener_handle = match &config.key_client {
            #[cfg(feature = "unix-client")]
            crate::config::KeyClientConfig::UnixSocket { .. } => {
                // Create temporary instance to start listener
                let temp_layer = Self {
                    key_cache: key_cache.clone(),
                    _key_listener_handle: None,
                };
                
                // Start unix socket listener for key pushes
                let handle = temp_layer.start_key_listener("/var/run/sequencer/keys.sock").await?;
                info!("Started key listener for unix socket key client");
                
                Some(handle)
            }
            crate::config::KeyClientConfig::Static { encryption_key, .. } => {
                // For static keys, populate the cache immediately
                let key_bytes = hex::decode(encryption_key)
                    .map_err(|e| EncryptionError::InvalidKeyFormat(format!("Invalid hex key: {e}")))?;
                let static_key = EncryptionKey {
                    id: "static-key".to_string(),
                    material: key_bytes,
                };
                key_cache.update_current_key(static_key);
                info!("Initialized with static encryption key");
                
                None
            }
        };
        
        Ok(Self { 
            key_cache,
            _key_listener_handle: key_listener_handle,
        })
    }

    // Main sync encryption function
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, String), EncryptionError> {
        let key = self.key_cache.get_current_key()
            .ok_or(EncryptionError::InvalidKeyFormat("No encryption key available".to_string()))?;
        let encrypted = self.encrypt_with_key(&key.material, plaintext)?;
        Ok((encrypted, key.id))
    }

    pub fn decrypt(&self, ciphertext: &[u8], _key_id: Option<&str>) -> Result<Vec<u8>, EncryptionError> {
        // For now, use current key for decryption
        let key = self.key_cache.get_current_key()
            .ok_or(EncryptionError::InvalidKeyFormat("No key available for decryption".to_string()))?;
        self.decrypt_with_key(&key.material, ciphertext)
    }
    
    #[cfg(feature = "aes-encryption")]
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

    #[cfg(feature = "aes-encryption")]
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
    
    // Start unix socket listener for pushed keys
    pub async fn start_key_listener<P: AsRef<Path> + Send + 'static>(&self, socket_path: P) -> Result<JoinHandle<()>, EncryptionError> {
        // Remove existing socket file if it exists
        let _ = std::fs::remove_file(&socket_path);
        
        let listener = UnixListener::bind(&socket_path)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("Socket bind failed: {e}")))?;
        let cache = self.key_cache.clone();
        
        let handle = tokio::spawn(async move {
            info!("Key listener started on {:?}", socket_path.as_ref());
            
            while let Ok((stream, _)) = listener.accept().await {
                let cache = cache.clone();
                
                // Handle each connection in a separate task
                tokio::spawn(async move {
                    if let Err(e) = Self::handle_key_connection(stream, cache).await {
                        error!("Error handling key connection: {}", e);
                    }
                });
            }
        });
        
        Ok(handle)
    }
    
    async fn handle_key_connection(
        mut stream: UnixStream, 
        cache: KeyCache
    ) -> Result<(), EncryptionError> {
        use tokio::io::AsyncReadExt;
        
        let mut buffer = vec![0u8; 4096];
        
        while let Ok(n) = stream.read(&mut buffer).await {
            if n == 0 { break; } // Connection closed
            
            // Deserialize key update message
            let key_update: KeyUpdate = bincode::deserialize(&buffer[..n])
                .map_err(|e| EncryptionError::EncryptionFailed(format!("Key update deserialization failed: {e}")))?;
            
            match key_update {
                KeyUpdate::NewKey(key) => {
                    info!("Received new encryption key: {}", key.id);
                    cache.update_current_key(key);
                }
                KeyUpdate::RotateKey { old_id, new_key } => {
                    info!("Key rotation: {} -> {}", old_id, new_key.id);
                    cache.rotate_key(old_id, new_key);
                }
                KeyUpdate::RevokeKey(key_id) => {
                    warn!("Key revoked: {}", key_id);
                    cache.revoke_key(key_id);
                }
            }
        }
        
        Ok(())
    }
}

impl EncryptionLayerTrait for EncryptionLayer {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        // Use the existing encrypt method but only return the ciphertext, not the key ID
        let (ciphertext, _key_id) = self.encrypt(plaintext)?;
        Ok(ciphertext)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        // Use the existing decrypt method with None for key_id
        self.decrypt(ciphertext, None)
    }

    fn encryption_type(&self) -> &'static str {
        "AES-256-GCM"
    }
}