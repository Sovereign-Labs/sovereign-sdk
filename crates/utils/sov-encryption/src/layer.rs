use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};
use tracing::{debug, error, info, warn};

#[cfg(feature = "aes-encryption")]
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
#[cfg(feature = "aes-encryption")]
use rand::{rngs::OsRng, RngCore};

use crate::{config::KeyClientConfig, error::EncryptionError};

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
        info!("🔑 KEY SET: '{}' for slot {}", key.id, slot_number);

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

// Remove the confusing trait - EncryptionLayer is now the primary interface

/// Primary encryption interface with slot-based key management
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
    /// Create a new encryption layer from a key client config (preferred)
    pub async fn new(key_client_config: KeyClientConfig) -> Result<Self, EncryptionError> {
        info!("Creating encryption layer with config: {:?}", key_client_config);
        let key_cache = Arc::new(KeyCache::new());

        // Handle different key client configurations
        let key_listener_handle = match &key_client_config {
            #[cfg(feature = "unix-client")]
            KeyClientConfig::UnixSocket {
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
            KeyClientConfig::Static { encryption_key, .. } => {
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
    fn encrypt_with_key(
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
    fn decrypt_with_key(
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
            use std::time::Duration;

            const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
            const MAX_BACKOFF: Duration = Duration::from_secs(30);
            const MAX_CONSECUTIVE_FAILURES: u32 = 10;

            let mut consecutive_failures: u32 = 0;
            let mut backoff_duration = INITIAL_BACKOFF;

            info!("🚀 Key listener started on {:?}", socket_path.as_ref());

            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        info!("🔌 New connection accepted on unix socket");

                        // Reset backoff on successful accept
                        consecutive_failures = 0;
                        backoff_duration = INITIAL_BACKOFF;

                        let cache = cache.clone();

                        // Handle each connection in a separate task
                        tokio::spawn(async move {
                            if let Err(e) = Self::handle_key_connection(stream, cache).await {
                                error!("❌ Error handling key connection: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        error!(
                            "❌ Failed to accept connection on unix socket (attempt {}/{}): {}",
                            consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
                        );

                        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                            error!(
                                "❌ Too many consecutive accept failures ({}), giving up",
                                consecutive_failures
                            );
                            break;
                        }

                        // Exponential backoff before retrying
                        warn!(
                            "⏳ Retrying accept in {:?}...",
                            backoff_duration
                        );
                        tokio::time::sleep(backoff_duration).await;
                        backoff_duration = (backoff_duration * 2).min(MAX_BACKOFF);
                        continue;
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
        stream: UnixStream,
        cache: Arc<KeyCache>,
    ) -> Result<(), EncryptionError> {
        debug!("New unix socket connection established for key updates");

        // Use length-delimited framing for reliable message boundaries
        let mut framed = FramedRead::new(stream, LengthDelimitedCodec::new());

        while let Some(result) = framed.next().await {
            match result {
                Ok(bytes) => {
                    debug!("Received {} bytes on unix socket (framed)", bytes.len());
                    debug!("Raw data: {:?}", &bytes[..]);

                    // Try to deserialize the key service bincode format
                    match Self::deserialize_key_service_message(&bytes) {
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
                            // Log error but continue processing
                            error!(
                                "❌ Failed to deserialize KeyUpdate message from {} bytes: {}",
                                bytes.len(),
                                e
                            );
                            debug!("Raw data hex: {}", hex::encode(&bytes[..]));
                            if let Ok(json_str) = std::str::from_utf8(&bytes[..]) {
                                debug!("Raw data as string: {}", json_str);
                            }
                            // Continue to next message instead of returning error
                            continue;
                        }
                    }
                }
                Err(e) => {
                    // Transport-level error - break the connection
                    error!("❌ Frame read error on unix socket: {}", e);
                    return Err(EncryptionError::EncryptionFailed(format!(
                        "Frame read error: {e}"
                    )));
                }
            }
        }

        info!("Unix socket connection closed by client");
        debug!("Unix socket key connection handler exiting");
        Ok(())
    }

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

    /// Get the key that would be used for a specific slot (for inspection/debugging)
    /// For actual encryption/decryption, use encrypt_for_slot() or decrypt_for_slot()
    pub fn get_key_for_slot(&self, slot_number: u64) -> Option<InternalKey> {
        let key = self.key_cache.get_key_for_slot(slot_number);

        if let Some(ref k) = key {
            debug!("🔑 INSPECT: Key '{}' available for slot {}", k.id, slot_number);
        } else {
            debug!("🔑 INSPECT: No key available for slot {}", slot_number);
        }

        key
    }

    /// Get the most recent available key from the cache
    fn get_most_recent_key(&self) -> Option<(u64, InternalKey)> {
        // First try the current key cache
        if let Some((slot, key)) = self.key_cache.current_key_cache.read().unwrap().as_ref() {
            return Some((*slot, key.clone()));
        }

        // Fall back to the most recent key in the map
        let map = self.key_cache.slot_key_map.read().unwrap();
        if let Some((slot, key)) = map.iter().next_back() {
            Some((*slot, key.clone()))
        } else {
            None
        }
    }

    /// Encrypt data for a specific slot with automatic fallback to most recent key
    /// Returns (encrypted_data, actual_slot_used)
    #[cfg(feature = "aes-encryption")]
    pub fn encrypt_for_slot(&self, slot_number: u64, plaintext: &[u8]) -> Result<(Vec<u8>, u64), EncryptionError> {
        // Try to get the key for the specific slot first
        if let Some(key) = self.get_key_for_slot(slot_number) {
            tracing::info!(
                "🔐 ENCRYPT: Using key '{}' for slot {} ({} bytes)",
                key.id,
                slot_number,
                plaintext.len()
            );
            let encrypted = self.encrypt_with_key(&key.material, plaintext)?;
            return Ok((encrypted, slot_number));
        }

        // Fallback: use the most recent available key
        if let Some((fallback_slot, fallback_key)) = self.get_most_recent_key() {
            tracing::warn!(
                "🔐 ENCRYPT FALLBACK: No key for slot {}, using most recent key '{}' from slot {}",
                slot_number,
                fallback_key.id,
                fallback_slot
            );
            let encrypted = self.encrypt_with_key(&fallback_key.material, plaintext)?;
            return Ok((encrypted, fallback_slot));
        }

        // No keys available at all
        Err(EncryptionError::InvalidKeyFormat(format!(
            "No encryption key available for slot {} or any fallback key",
            slot_number
        )))
    }

    /// Decrypt data for a specific slot with automatic fallback to other available keys
    #[cfg(feature = "aes-encryption")]
    pub fn decrypt_for_slot(&self, slot_number: u64, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        // Try to get the key for the specific slot first
        if let Some(key) = self.get_key_for_slot(slot_number) {
            tracing::debug!(
                "🔓 DECRYPT: Using key '{}' for slot {} ({} bytes)",
                key.id,
                slot_number,
                ciphertext.len()
            );
            return self.decrypt_with_key(&key.material, ciphertext);
        }

        // Fallback: try all available keys (the ciphertext might have been encrypted with a different key)
        tracing::warn!(
            "🔓 DECRYPT FALLBACK: No key for slot {}, trying all available keys",
            slot_number
        );
        
        let map = self.key_cache.slot_key_map.read().unwrap();
        for (try_slot, key) in map.iter().rev() { // Try most recent keys first
            if let Ok(plaintext) = self.decrypt_with_key(&key.material, ciphertext) {
                tracing::info!(
                    "🔓 DECRYPT SUCCESS: Key '{}' from slot {} successfully decrypted data for slot {}",
                    key.id,
                    try_slot,
                    slot_number
                );
                return Ok(plaintext);
            }
        }

        // No key could decrypt the data
        Err(EncryptionError::DecryptionFailed(format!(
            "No available key could decrypt the data for slot {}",
            slot_number
        )))
    }
}
