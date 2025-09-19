use std::sync::{Arc, RwLock};
use std::fmt;
use std::path::Path;
use std::time::Duration;

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
    config::{CipherType, EncryptionConfig},
    error::EncryptionError,
    key_client::{KeyClient, create_key_client},
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

#[derive(Debug, Serialize, Deserialize)]
pub enum KeyManagerRequest {
    GetCurrentKey,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum KeyManagerResponse {
    Key(EncryptionKey),
    Error(String),
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

#[cfg(feature = "aes-encryption")]
#[allow(dead_code)]
pub struct AesGcmEncryptionLayer {
    key_client: Arc<Box<dyn KeyClient>>,
}

#[cfg(feature = "aes-encryption")]
#[allow(dead_code)]
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
impl EncryptionLayerTrait for AesGcmEncryptionLayer {
    fn encrypt(&self, _plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        // This old implementation is async-based, but we're moving to sync with key cache
        // For now, return an error directing to use the new EncryptionLayer
        Err(EncryptionError::EncryptionFailed(
            "Use EncryptionLayer with key cache instead of AesGcmEncryptionLayer directly".to_string()
        ))
    }

    fn decrypt(&self, _ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        // This old implementation is async-based, but we're moving to sync with key cache
        // For now, return an error directing to use the new EncryptionLayer
        Err(EncryptionError::DecryptionFailed(
            "Use EncryptionLayer with key cache instead of AesGcmEncryptionLayer directly".to_string()
        ))
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
    key_cache: KeyCache,
    keyclient_socket_path: std::path::PathBuf,
    _key_listener_handle: Option<JoinHandle<()>>,
}

impl EncryptionLayer {
    pub async fn new(config: EncryptionConfig) -> Result<Self, EncryptionError> {
        let key_cache = KeyCache::new();
        
        // Extract keyclient socket path from config and handle static keys
        let (keyclient_socket_path, key_listener_handle) = match &config.key_client {
            #[cfg(feature = "unix-client")]
            crate::config::KeyClientConfig::UnixSocket { socket_path, .. } => {
                // Create temporary instance to start listener
                let temp_layer = Self {
                    key_cache: key_cache.clone(),
                    keyclient_socket_path: socket_path.clone(),
                    _key_listener_handle: None,
                };
                
                // Start unix socket listener for key pushes
                let handle = temp_layer.start_key_listener("/var/run/sequencer/keys.sock").await?;
                info!("Started key listener for unix socket key client");
                
                (socket_path.clone(), Some(handle))
            }
            crate::config::KeyClientConfig::Static { encryption_key, .. } => {
                // For static keys, populate the cache immediately and use dummy socket path
                let key_bytes = hex::decode(encryption_key)
                    .map_err(|e| EncryptionError::InvalidKeyFormat(format!("Invalid hex key: {e}")))?;
                let static_key = EncryptionKey {
                    id: "static-key".to_string(),
                    material: key_bytes,
                };
                key_cache.update_current_key(static_key);
                info!("Initialized with static encryption key");
                
                // Use dummy path since static keys don't need socket communication
                (std::path::PathBuf::from("/tmp/dummy-keyclient.sock"), None)
            }
            #[cfg(feature = "http-client")]
            crate::config::KeyClientConfig::Http { .. } => {
                // For HTTP client, use default path
                info!("Using HTTP key client - no listener needed");
                (std::path::PathBuf::from("/tmp/keyclient.sock"), None)
            }
        };
        
        Ok(Self { 
            key_cache,
            keyclient_socket_path,
            _key_listener_handle: key_listener_handle,
        })
    }

    // Main sync encryption function with fallback
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, String), EncryptionError> {
        // Try cached key first (fast path - 99% of calls)
        if let Some(key) = self.key_cache.get_current_key() {
            let encrypted = self.encrypt_with_key(&key.material, plaintext)?;
            return Ok((encrypted, key.id));
        }
        
        // Cache miss - emergency sync fetch (rare fallback)
        warn!("Key cache miss, fetching from keyclient synchronously");
        let key = self.fetch_key_sync()?;
        self.key_cache.update_current_key(key.clone());
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
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
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
    
    // Sync fallback - blocking call to keyclient
    fn fetch_key_sync(&self) -> Result<EncryptionKey, EncryptionError> {
        use std::os::unix::net::UnixStream;
        use std::io::{Read, Write};
        
        let mut stream = UnixStream::connect(&self.keyclient_socket_path)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("KeyClient connection failed: {e}")))?;
        
        // Send key request
        let request = KeyManagerRequest::GetCurrentKey;
        let request_bytes = bincode::serialize(&request)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("Request serialization failed: {e}")))?;
        stream.write_all(&request_bytes)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("Request write failed: {e}")))?;
        
        // Read response with timeout
        stream.set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| EncryptionError::EncryptionFailed(format!("Timeout set failed: {e}")))?;
        let mut response_buffer = Vec::new();
        stream.read_to_end(&mut response_buffer)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("Response read failed: {e}")))?;
        
        let response: KeyManagerResponse = bincode::deserialize(&response_buffer)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("Response deserialization failed: {e}")))?;
        match response {
            KeyManagerResponse::Key(key) => Ok(key),
            KeyManagerResponse::Error(msg) => Err(EncryptionError::EncryptionFailed(format!("KeyClient error: {msg}"))),
        }
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