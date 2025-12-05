use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use parking_lot::RwLock;

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

/// Number of old keys to keep as a buffer when pruning after decryption.
/// Example: with buffer_size=2 and keys [A, B, C, D], decrypting with D removes A → [B, C, D]
const KEY_PRUNE_BUFFER_SIZE: usize = 2;

#[derive(Clone)]
pub struct KeyCache {
    // Queue of keys in arrival order (oldest first, newest last)
    // Each entry is (slot, key) where slot is the slot number the key is valid from
    key_queue: Arc<RwLock<VecDeque<(u64, InternalKey)>>>,
}

impl KeyCache {
    pub fn new() -> Self {
        Self {
            key_queue: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// For ENCRYPTION: Find key by slot logic with fallback to most recent.
    /// Returns the newest key where key_slot <= batch_slot, or the most recent key as fallback.
    pub fn get_key_for_slot(&self, slot_number: u64) -> Option<InternalKey> {
        let queue = self.key_queue.read();

        // Primary: find newest key where slot <= batch_slot
        for (slot, key) in queue.iter().rev() {
            if *slot <= slot_number {
                debug!(
                    "🔑 Found key '{}' (slot {}) for batch slot {}",
                    key.id, slot, slot_number
                );
                return Some(key.clone());
            }
        }

        // Fallback: use the most recent key (all keys have slots > batch_slot)
        if let Some((slot, key)) = queue.back() {
            debug!(
                "🔑 FALLBACK: No key with slot <= {}, using most recent '{}' (slot {})",
                slot_number, key.id, slot
            );
            return Some(key.clone());
        }

        debug!("🔑 NO KEY: No keys available");
        None
    }

    /// For DECRYPTION: Get exact key by ID.
    pub fn get_key_by_id(&self, key_id: &str) -> Option<InternalKey> {
        self.key_queue
            .read()
            .iter()
            .find(|(_, key)| key.id == key_id)
            .map(|(_, key)| key.clone())
    }

    /// Add a new key to the cache.
    pub fn add_key(&self, slot_number: u64, key: InternalKey) {
        info!("🔑 KEY ADDED: '{}' for slot {}", key.id, slot_number);
        let mut queue = self.key_queue.write();
        queue.push_back((slot_number, key));
    }

    /// Prune old keys, keeping a buffer of N keys before the specified key.
    /// Example: with buffer_size=2 and keys [A, B, C, D], pruning at D removes A → [B, C, D]
    pub fn prune_keys_before_id(&self, key_id: &str, buffer_size: usize) {
        let mut queue = self.key_queue.write();

        // Find the position of the key we just used
        let Some(pos) = queue.iter().position(|(_, k)| k.id == key_id) else {
            return;
        };

        // Keep buffer_size keys before the used key, remove older ones
        let remove_count = pos.saturating_sub(buffer_size);
        if remove_count > 0 {
            queue.drain(0..remove_count);
            info!(
                "🗑️ PRUNED: Removed {} old key(s), {} remaining",
                remove_count,
                queue.len()
            );
        }
    }

    /// Get the number of keys currently in the cache
    pub fn len(&self) -> usize {
        self.key_queue.read().len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.key_queue.read().is_empty()
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
                    key_cache.add_key(0, initial_encryption_key);
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
                key_cache.add_key(0, static_key);
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
        use std::os::unix::fs::PermissionsExt;

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

        // Set restrictive permissions: only owner can read/write (0600)
        // This ensures only processes running as the same UID can connect
        std::fs::set_permissions(
            socket_path.as_ref(),
            std::fs::Permissions::from_mode(0o600),
        )
        .map_err(|e| {
            error!("❌ SOCKET PERMISSIONS FAILED: Cannot set permissions on {:?}: {}", socket_path.as_ref(), e);
            EncryptionError::EncryptionFailed(format!("Socket permissions failed: {e}"))
        })?;
        info!("🔒 Socket permissions set to 0600 (owner only)");

        let cache = self.key_cache.clone();

        let handle = tokio::spawn(async move {
            use std::time::Duration;

            const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
            const MAX_BACKOFF: Duration = Duration::from_secs(30);
            const MAX_CONSECUTIVE_FAILURES: u32 = 10;

            let mut consecutive_failures: u32 = 0;
            let mut backoff_duration = INITIAL_BACKOFF;

            info!("🚀 Key listener started on {:?} (single connection mode)", socket_path.as_ref());

            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        info!("🔌 Connection accepted on unix socket (blocking until closed)");

                        // Reset backoff on successful accept
                        consecutive_failures = 0;
                        backoff_duration = INITIAL_BACKOFF;

                        // Handle this ONE connection - blocks accepting new ones until it closes
                        // This ensures only one key service can be connected at a time
                        if let Err(e) = Self::handle_key_connection(stream, cache.clone()).await {
                            error!("❌ Error handling key connection: {}", e);
                        }

                        info!("🔌 Connection closed, ready to accept next connection");
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

                                    cache.add_key(target_slot, internal_key);
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
        let queue = self.key_cache.key_queue.read();

        if queue.is_empty() {
            warn!("🔍 KEY STATUS: No keys available");
        } else {
            info!("🔍 KEY STATUS: {} keys in queue:", queue.len());

            // Show current (newest) key
            if let Some((slot, key)) = queue.back() {
                info!("💾 CURRENT KEY: Slot {} -> Key '{}'", slot, key.id);
            }

            // Show keys in queue (oldest first)
            let display_count = queue.len().min(5);
            info!("🔍 Queue (oldest first):");
            for (slot, key) in queue.iter().take(display_count) {
                info!("🔍   - Slot {}: Key '{}'", slot, key.id);
            }

            if queue.len() > 5 {
                info!("🔍   ... and {} more keys", queue.len() - 5);
            }
        }
    }

    /// Get the key that would be used for a specific slot (for inspection/debugging)
    /// For actual encryption/decryption, use encrypt_for_slot() or decrypt_with_key_id()
    pub fn get_key_for_slot(&self, slot_number: u64) -> Option<InternalKey> {
        let key = self.key_cache.get_key_for_slot(slot_number);

        if let Some(ref k) = key {
            debug!("🔑 INSPECT: Key '{}' available for slot {}", k.id, slot_number);
        } else {
            debug!("🔑 INSPECT: No key available for slot {}", slot_number);
        }

        key
    }

    /// Encrypt data for a specific slot.
    /// Returns (encrypted_data, key_id) where key_id identifies which key was used.
    /// 
    /// Key selection logic:
    /// 1. Find newest key where key_slot <= batch_slot
    /// 2. Fallback: use the most recent key if all keys have slots > batch_slot
    #[cfg(feature = "aes-encryption")]
    pub fn encrypt_for_slot(&self, slot_number: u64, plaintext: &[u8]) -> Result<(Vec<u8>, String), EncryptionError> {
        let key = self.key_cache.get_key_for_slot(slot_number)
            .ok_or_else(|| EncryptionError::InvalidKeyFormat(
                "No encryption key available".into()
            ))?;

        tracing::info!(
            "🔐 ENCRYPT: Using key '{}' for batch slot {} ({} bytes)",
            key.id,
            slot_number,
            plaintext.len()
        );

        let encrypted = self.encrypt_with_key(&key.material, plaintext)?;
        Ok((encrypted, key.id))
    }

    /// Decrypt data using the specific key ID that was used for encryption.
    /// Falls back to trying all keys if the specified key is not found or fails.
    /// 
    /// Automatically prunes old keys after successful decryption, keeping KEY_PRUNE_BUFFER_SIZE
    /// keys as a buffer before the used key.
    #[cfg(feature = "aes-encryption")]
    pub fn decrypt_with_key_id(
        &self,
        key_id: &str,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        // Primary: try the exact key by ID
        if let Some(key) = self.key_cache.get_key_by_id(key_id) {
            match self.decrypt_with_key(&key.material, ciphertext) {
                Ok(plaintext) => {
                    tracing::debug!(
                        "🔓 DECRYPT: Successfully decrypted with key '{}' ({} bytes)",
                        key_id,
                        ciphertext.len()
                    );

                    // Prune old keys, keeping buffer
                    self.key_cache.prune_keys_before_id(key_id, KEY_PRUNE_BUFFER_SIZE);

                    return Ok(plaintext);
                }
                Err(e) => {
                    tracing::warn!(
                        "🔓 Key '{}' found but decryption failed: {}, trying fallback",
                        key_id,
                        e
                    );
                }
            }
        } else {
            tracing::warn!("🔓 Key '{}' not found, trying fallback", key_id);
        }

        // Fallback: brute force try all keys, newest first
        tracing::warn!("🔓 DECRYPT FALLBACK: Trying all available keys");

        let queue = self.key_cache.key_queue.read();
        for (_, key) in queue.iter().rev() {
            if let Ok(plaintext) = self.decrypt_with_key(&key.material, ciphertext) {
                let used_key_id = key.id.clone();
                drop(queue); // Release read lock before pruning

                tracing::info!(
                    "🔓 FALLBACK SUCCESS: Decrypted with key '{}'",
                    used_key_id
                );

                // Prune old keys, keeping buffer
                self.key_cache.prune_keys_before_id(&used_key_id, KEY_PRUNE_BUFFER_SIZE);

                return Ok(plaintext);
            }
        }
        drop(queue);

        Err(EncryptionError::DecryptionFailed(
            "No available key could decrypt the data".into()
        ))
    }

    /// Get a key by its ID (for inspection/debugging)
    pub fn get_key_by_id(&self, key_id: &str) -> Option<InternalKey> {
        self.key_cache.get_key_by_id(key_id)
    }
}
