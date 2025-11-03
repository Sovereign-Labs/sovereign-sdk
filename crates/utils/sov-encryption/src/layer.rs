use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

#[cfg(feature = "aes-encryption")]
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
#[cfg(feature = "aes-encryption")]
use rand::{rngs::OsRng, RngCore};

use crate::{config::EncryptionConfig, error::EncryptionError};

#[cfg(feature = "aes-encryption")]
const AES_256_KEY_SIZE: usize = 32;
#[cfg(feature = "aes-encryption")]
const AES_GCM_NONCE_SIZE: usize = 12;

/// Key format from the key service (matches your service exactly)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionKey {
    pub id: String,
    pub key_data: Vec<u8>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub slot_number: u64,
}

/// Internal key format used by the encryption layer
#[derive(Debug, Clone)]
pub struct InternalKey {
    pub id: String,
    pub material: Vec<u8>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyUpdate {
    NewKey(EncryptionKey),
}

#[derive(Clone)]
pub struct KeyCache {
    // Map of slot number to encryption key (sorted for efficient range queries)
    slot_key_map: Arc<RwLock<BTreeMap<u64, InternalKey>>>,
    // Cache of the most recent key for O(1) access in common case
    current_key_cache: Arc<RwLock<Option<(u64, InternalKey)>>>,
}

impl KeyCache {
    pub fn new() -> Self {
        Self {
            slot_key_map: Arc::new(RwLock::new(BTreeMap::new())),
            current_key_cache: Arc::new(RwLock::new(None)),
        }
    }

    pub fn get_key_for_slot(&self, slot_number: u64) -> Option<InternalKey> {
        // Fast path: check if current key cache works (O(1))
        if let Some((cached_slot, cached_key)) = self.current_key_cache.read().unwrap().as_ref() {
            if slot_number >= *cached_slot {
                debug!(
                    "🔑 FAST PATH: Using cached key '{}' (slot {}) for slot {}",
                    cached_key.id, cached_slot, slot_number
                );
                return Some(cached_key.clone());
            }
        }

        // Slow path: historical lookup in BTreeMap (O(log n))
        let map = self.slot_key_map.read().unwrap();
        if let Some((_, key)) = map.range(..=slot_number).next_back() {
            debug!(
                "🔑 SLOW PATH: Historical lookup for slot {} found key '{}'",
                slot_number, key.id
            );
            Some(key.clone())
        } else {
            debug!("🔑 NO KEY: No key available for slot {}", slot_number);
            None
        }
    }

    pub fn set_key_for_slot(&self, slot_number: u64, key: InternalKey) {
        info!(
            "🔑 KEY SET: '{}' for slot {}",
            key.id, slot_number
        );
        
        // Store in the main map
        self.slot_key_map
            .write()
            .unwrap()
            .insert(slot_number, key.clone());
        
        // Update current key cache if this is the newest key
        let mut current_cache = self.current_key_cache.write().unwrap();
        let should_update = match current_cache.as_ref() {
            Some((cached_slot, _)) => slot_number >= *cached_slot,
            None => true,
        };
        
        if should_update {
            *current_cache = Some((slot_number, key));
            debug!(
                "💾 CACHE UPDATE: Updated current key cache for slot {}",
                slot_number
            );
        }
    }






    pub fn revoke_key(&self, key_id: String) {
        let mut map = self.slot_key_map.write().unwrap();
        map.retain(|_, key| key.id != key_id);
        
        // Clear current cache if it contains the revoked key
        let mut current_cache = self.current_key_cache.write().unwrap();
        if let Some((_, cached_key)) = current_cache.as_ref() {
            if cached_key.id == key_id {
                *current_cache = None;
                warn!("Current key cache cleared due to key revocation");
            }
        }
        
        warn!("Key '{}' revoked from all slots", key_id);
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
    key_cache: Arc<KeyCache>,
    _key_listener_handle: Option<JoinHandle<()>>,
}

impl Clone for EncryptionLayer {
    fn clone(&self) -> Self {
        Self {
            key_cache: self.key_cache.clone(),
            _key_listener_handle: None, // Don't clone the handle - only one listener should exist
        }
    }
}

impl EncryptionLayer {
    pub async fn new(config: EncryptionConfig) -> Result<Self, EncryptionError> {
        info!("Creating encryption layer with config: {:?}", config);
        let key_cache = Arc::new(KeyCache::new());

        // Handle different key client configurations
        let key_listener_handle = match &config.key_client {
            #[cfg(feature = "unix-client")]
            crate::config::KeyClientConfig::UnixSocket {
                socket_path,
                initial_key,
                ..
            } => {
                // If an initial key is provided, populate the cache with it
                if let Some(key_hex) = initial_key {
                    let key_bytes = hex::decode(key_hex).map_err(|e| {
                        EncryptionError::InvalidKeyFormat(format!("Invalid hex initial key: {e}"))
                    })?;
                    if key_bytes.len() != 32 {
                        return Err(EncryptionError::InvalidKeyFormat(format!(
                            "Initial key must be 32 bytes, got {}",
                            key_bytes.len()
                        )));
                    }
                    let initial_encryption_key = InternalKey {
                        id: "genesis-key".to_string(),
                        material: key_bytes,
                    };
                    key_cache.set_key_for_slot(0, initial_encryption_key);
                    info!("Initialized unix socket encryption layer with genesis key");
                }

                // Create temporary instance to start listener
                let temp_layer = Self {
                    key_cache: key_cache.clone(),
                    _key_listener_handle: None,
                };

                // Clone the socket path to satisfy lifetime requirements
                let socket_path_cloned = socket_path.clone();

                // Start unix socket listener for key pushes
                let handle = temp_layer.start_key_listener(socket_path_cloned).await?;
                info!(
                    "Started key listener for unix socket key client at {:?}",
                    socket_path
                );

                Some(handle)
            }
            crate::config::KeyClientConfig::Static { encryption_key, .. } => {
                // For static keys, populate the cache immediately
                let key_bytes = hex::decode(encryption_key).map_err(|e| {
                    EncryptionError::InvalidKeyFormat(format!("Invalid hex key: {e}"))
                })?;
                let static_key = InternalKey {
                    id: "static-key".to_string(),
                    material: key_bytes,
                };
                key_cache.set_key_for_slot(0, static_key);
                info!("Initialized with static encryption key");

                None
            }
        };

        Ok(Self {
            key_cache,
            _key_listener_handle: key_listener_handle,
        })
    }

    #[cfg(feature = "aes-encryption")]
    pub fn encrypt_with_key(
        &self,
        key: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        if key.len() != AES_256_KEY_SIZE {
            return Err(EncryptionError::InvalidKeyFormat(format!(
                "Expected {} byte key, got {}",
                AES_256_KEY_SIZE,
                key.len()
            )));
        }

        let cipher_key = Key::<Aes256Gcm>::from_slice(key);
        let cipher = Aes256Gcm::new(cipher_key);

        // Generate a random nonce
        let mut nonce_bytes = [0u8; AES_GCM_NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        debug!("Encrypting {} bytes with AES-256-GCM", plaintext.len());

        let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|e| {
            EncryptionError::EncryptionFailed(format!("AES-GCM encryption failed: {e}"))
        })?;

        // Prepend nonce to ciphertext for storage
        let mut result = Vec::with_capacity(AES_GCM_NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        debug!(
            "Successfully encrypted to {} bytes (including nonce)",
            result.len()
        );
        Ok(result)
    }

    #[cfg(feature = "aes-encryption")]
    pub fn decrypt_with_key(
        &self,
        key: &[u8],
        ciphertext_with_nonce: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        if key.len() != AES_256_KEY_SIZE {
            return Err(EncryptionError::InvalidKeyFormat(format!(
                "Expected {} byte key, got {}",
                AES_256_KEY_SIZE,
                key.len()
            )));
        }

        if ciphertext_with_nonce.len() < AES_GCM_NONCE_SIZE {
            return Err(EncryptionError::InvalidCiphertextFormat(format!(
                "Ciphertext too short: expected at least {} bytes, got {}",
                AES_GCM_NONCE_SIZE,
                ciphertext_with_nonce.len()
            )));
        }

        let cipher_key = Key::<Aes256Gcm>::from_slice(key);
        let cipher = Aes256Gcm::new(cipher_key);

        // Extract nonce from the beginning
        let (nonce_bytes, ciphertext) = ciphertext_with_nonce.split_at(AES_GCM_NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        debug!("Decrypting {} bytes with AES-256-GCM", ciphertext.len());

        let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|e| {
            EncryptionError::DecryptionFailed(format!("AES-GCM decryption failed: {e}"))
        })?;

        debug!("Successfully decrypted to {} bytes", plaintext.len());
        Ok(plaintext)
    }

    // Start unix socket listener for pushed keys
    pub async fn start_key_listener<P: AsRef<Path> + Send + 'static>(
        &self,
        socket_path: P,
    ) -> Result<JoinHandle<()>, EncryptionError> {
        // Remove existing socket file if it exists
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)
            .map_err(|e| {
                error!("❌ SOCKET BIND FAILED: Cannot bind to {:?}: {}", socket_path.as_ref(), e);
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    error!("❌ ADDRESS IN USE: Another process is already using this socket path!");
                    error!("❌ SOLUTION: Use different socket paths for sequencer and STF, or share the encryption layer");
                }
                EncryptionError::EncryptionFailed(format!("Socket bind failed: {e}"))
            })?;
        let cache = self.key_cache.clone();

        let handle = tokio::spawn(async move {
            info!("🚀 Key listener started on {:?}", socket_path.as_ref());

            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        info!("🔌 New connection accepted on unix socket");
                        let cache = cache.clone();

                        // Handle each connection in a separate task
                        tokio::spawn(async move {
                            if let Err(e) = Self::handle_key_connection(stream, cache).await {
                                error!("❌ Error handling key connection: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("❌ Failed to accept connection on unix socket: {}", e);
                        break;
                    }
                }
            }
            warn!("🛑 Key listener exiting");
        });

        Ok(handle)
    }

    /// Deserialize the key service message format
    fn deserialize_key_service_message(data: &[u8]) -> Result<KeyUpdate, String> {
        // Try to deserialize directly since the formats should match now
        match bincode::deserialize::<KeyUpdate>(data) {
            Ok(key_update) => {
                debug!("📋 PARSED KEY SERVICE: Successfully deserialized KeyUpdate");
                Ok(key_update)
            }
            Err(e) => Err(format!("Failed to deserialize KeyUpdate: {}", e)),
        }
    }

    async fn handle_key_connection(
        mut stream: UnixStream,
        cache: Arc<KeyCache>,
    ) -> Result<(), EncryptionError> {
        use tokio::io::AsyncReadExt;

        debug!("New unix socket connection established for key updates");
        let mut buffer = vec![0u8; 4096];

        while let Ok(n) = stream.read(&mut buffer).await {
            if n == 0 {
                info!("Unix socket connection closed by client");
                break;
            }

            debug!("Received {} bytes on unix socket", n);
            debug!("Raw data: {:?}", &buffer[..n]);

            // Try to deserialize the key service bincode format
            let key_update_result = Self::deserialize_key_service_message(&buffer[..n]);

            match key_update_result {
                Ok(key_update) => {
                    debug!("Successfully deserialized KeyUpdate message");
                    match key_update {
                        KeyUpdate::NewKey(key) => {
                            info!(
                                "📨 KEY RECEIVED: NEW encryption key '{}' ({} bytes) for slot {}",
                                key.id,
                                key.key_data.len(),
                                key.slot_number
                            );

                            let target_slot = key.slot_number;

                            // Convert to internal format and store directly
                            let internal_key = InternalKey {
                                id: key.id.clone(),
                                material: key.key_data.clone(),
                            };

                            cache.set_key_for_slot(target_slot, internal_key);
                            info!(
                                "✅ KEY STORED: Key '{}' stored for slot {}",
                                key.id, target_slot
                            );
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "❌ Failed to deserialize KeyUpdate message from {} bytes: {}",
                        n, e
                    );
                    debug!("Raw data hex: {}", hex::encode(&buffer[..n]));
                    if let Ok(json_str) = std::str::from_utf8(&buffer[..n]) {
                        debug!("Raw data as string: {}", json_str);
                    }
                    return Err(EncryptionError::EncryptionFailed(format!(
                        "Key update deserialization failed: {e}"
                    )));
                }
            }
        }

        debug!("Unix socket key connection handler exiting");
        Ok(())
    }
}

impl EncryptionLayer {
    /// Debug method to show key status
    pub fn debug_key_status(&self) {
        let map = self.key_cache.slot_key_map.read().unwrap();
        let current_cache = self.key_cache.current_key_cache.read().unwrap();
        
        if map.is_empty() {
            warn!("🔍 KEY STATUS: No keys available");
        } else {
            info!("🔍 KEY STATUS: {} keys stored:", map.len());
            
            // Show current cache status
            match current_cache.as_ref() {
                Some((slot, key)) => {
                    info!("💾 CURRENT CACHE: Slot {} -> Key '{}'", slot, key.id);
                }
                None => {
                    info!("💾 CURRENT CACHE: Empty");
                }
            }
            
            // Show recent keys (last 5)
            let slots: Vec<_> = map.keys().rev().take(5).cloned().collect();
            for slot in slots {
                if let Some(key) = map.get(&slot) {
                    info!("🔍   - Slot {}: Key '{}'", slot, key.id);
                }
            }
            
            if map.len() > 5 {
                info!("🔍   ... and {} more historical keys", map.len() - 5);
            }
        }
    }

    /// Get the appropriate key for a specific slot number
    /// This ensures sequencer and STF use the same key for the same slot
    pub fn get_key_for_slot(&self, slot_number: u64) -> Option<InternalKey> {
        let key = self.key_cache.get_key_for_slot(slot_number);
        
        if let Some(ref k) = key {
            debug!(
                "🔑 SLOT KEY: Using key '{}' for slot {}",
                k.id, slot_number
            );
        } else {
            warn!("🔑 SLOT KEY: No key available for slot {}", slot_number);
        }

        key
    }





}

impl EncryptionLayerTrait for EncryptionLayer {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        // For trait methods, we need a slot number. We'll use slot 0 as default
        // In practice, callers should use get_key_for_slot() + encrypt_with_key() directly
        let key = self.get_key_for_slot(0)
            .ok_or(EncryptionError::InvalidKeyFormat(
                "No encryption key available for slot 0".to_string(),
            ))?;

        warn!(
            "🔐 ENCRYPT: Using key '{}' for {} bytes (trait method - consider using get_key_for_slot + encrypt_with_key)",
            key.id,
            plaintext.len()
        );

        self.encrypt_with_key(&key.material, plaintext)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        // For trait methods, we need a slot number. We'll use slot 0 as default
        // In practice, callers should use get_key_for_slot() + decrypt_with_key() directly
        let key = self.get_key_for_slot(0)
            .ok_or(EncryptionError::InvalidKeyFormat(
                "No key available for decryption at slot 0".to_string(),
            ))?;

        warn!(
            "🔓 DECRYPT: Using key '{}' for {} bytes (trait method - consider using get_key_for_slot + decrypt_with_key)",
            key.id,
            ciphertext.len()
        );

        self.decrypt_with_key(&key.material, ciphertext)
    }

    fn encryption_type(&self) -> &'static str {
        "AES-256-GCM"
    }
}
