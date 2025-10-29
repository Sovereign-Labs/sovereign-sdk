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
    #[serde(alias = "key_data")]
    pub material: Vec<u8>,
    #[serde(default)]
    pub slot_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotScheduledKey {
    pub key: EncryptionKey,
    pub activate_at_slot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyUpdate {
    NewKey(EncryptionKey),
    ScheduledKey(SlotScheduledKey),
    RotateKey { old_id: String, new_key: EncryptionKey },
    RevokeKey(String),
}

#[derive(Clone)]
pub struct KeyCache {
    // Arc<> - Allows multiple threads to access the same key safely
    //
    // RwLock<> Allows readers to hold the lock simultaneously and 1 writer
    // More performant than Mutex for cases where data is read frequently and written less frequently

    // RwLock<> could result in writer starvation if there is constant stream of readers
    // Writer could be blocked indefinitely so we'll never update the key. 
    current_key: Arc<RwLock<Option<EncryptionKey>>>,
    scheduled_keys: Arc<RwLock<Vec<SlotScheduledKey>>>,
    current_slot: Arc<RwLock<u64>>,
}

impl KeyCache {
    pub fn new() -> Self {
        Self {
            current_key: Arc::new(RwLock::new(None)),
            scheduled_keys: Arc::new(RwLock::new(Vec::new())),
            current_slot: Arc::new(RwLock::new(0)),
        }
    }
    
    pub fn get_current_key(&self) -> Option<EncryptionKey> {
        self.current_key.read().unwrap().clone()
    }
    
    pub fn update_current_key(&self, key: EncryptionKey) {
        *self.current_key.write().unwrap() = Some(key);
    }
    
    pub fn schedule_key(&self, scheduled_key: SlotScheduledKey) {
        info!("📅 Scheduling key {} for activation at slot {}", scheduled_key.key.id, scheduled_key.activate_at_slot);
        let mut scheduled = self.scheduled_keys.write().unwrap();
        
        // Insert in sorted order by activation slot
        let pos = scheduled.binary_search_by_key(&scheduled_key.activate_at_slot, |k| k.activate_at_slot)
            .unwrap_or_else(|e| e);
        scheduled.insert(pos, scheduled_key);
        
        info!("📊 Total scheduled keys: {}", scheduled.len());
    }
    
    pub fn update_slot(&self, slot_number: u64) -> Vec<EncryptionKey> {
        let mut activated_keys = Vec::new();
        
        // Update current slot
        *self.current_slot.write().unwrap() = slot_number;
        
        // Check for keys to activate
        activated_keys.extend(self.check_for_activation(slot_number));
        
        activated_keys
    }
    
    /// Check for keys that should be activated at the given slot number without updating current slot
    /// This allows proactive activation without blocking
    pub fn check_for_activation(&self, slot_number: u64) -> Vec<EncryptionKey> {
        let mut activated_keys = Vec::new();
        let mut scheduled = self.scheduled_keys.write().unwrap();
        let mut to_activate = Vec::new();
        
        // Find all keys that should be activated at this slot
        while let Some(scheduled_key) = scheduled.first() {
            if scheduled_key.activate_at_slot <= slot_number {
                to_activate.push(scheduled.remove(0));
            } else {
                break;
            }
        }
        
        // Activate keys (most recent one becomes current)
        for scheduled_key in to_activate {
            info!("🔑 KEY ACTIVATION: Activating key '{}' for slot {} (was scheduled for slot {})", 
                  scheduled_key.key.id, slot_number, scheduled_key.activate_at_slot);
            debug!("🔑 KEY ACTIVATION: Key material hash: {}", 
                   hex::encode(&scheduled_key.key.material[..8.min(scheduled_key.key.material.len())]));
            activated_keys.push(scheduled_key.key.clone());
            self.update_current_key(scheduled_key.key);
        }
        
        if !activated_keys.is_empty() {
            info!("📊 KEY STATUS: {} keys activated, {} keys remaining scheduled", 
                  activated_keys.len(), scheduled.len());
        }
        
        activated_keys
    }
    
    pub fn get_current_slot(&self) -> u64 {
        *self.current_slot.read().unwrap()
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
        
        // Also remove from scheduled keys
        let mut scheduled = self.scheduled_keys.write().unwrap();
        scheduled.retain(|sk| sk.key.id != key_id);
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
        info!("Creating encryption layer with config: {:?}", config);
        let key_cache = KeyCache::new();
        
        // Handle different key client configurations
        let key_listener_handle = match &config.key_client {
            #[cfg(feature = "unix-client")]
            crate::config::KeyClientConfig::UnixSocket { socket_path, initial_key, .. } => {
                // If an initial key is provided, populate the cache with it
                if let Some(key_hex) = initial_key {
                    let key_bytes = hex::decode(key_hex)
                        .map_err(|e| EncryptionError::InvalidKeyFormat(format!("Invalid hex initial key: {e}")))?;
                    if key_bytes.len() != 32 {
                        return Err(EncryptionError::InvalidKeyFormat(
                            format!("Initial key must be 32 bytes, got {}", key_bytes.len())
                        ));
                    }
                    let initial_encryption_key = EncryptionKey {
                        id: "genesis-key".to_string(),
                        material: key_bytes,
                        slot_number: None,
                        timestamp: None,
                    };
                    key_cache.update_current_key(initial_encryption_key);
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
                info!("Started key listener for unix socket key client at {:?}", socket_path);
                
                Some(handle)
            }
            crate::config::KeyClientConfig::Static { encryption_key, .. } => {
                // For static keys, populate the cache immediately
                let key_bytes = hex::decode(encryption_key)
                    .map_err(|e| EncryptionError::InvalidKeyFormat(format!("Invalid hex key: {e}")))?;
                let static_key = EncryptionKey {
                    id: "static-key".to_string(),
                    material: key_bytes,
                    slot_number: None,
                    timestamp: None,
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
    
    /// Deserialize the key service message format with proper compatibility
    fn deserialize_key_service_message(data: &[u8]) -> Result<KeyUpdate, String> {
        // Define the key service's EncryptionKey format for compatibility
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct KeyServiceEncryptionKey {
            pub id: String,
            pub key_data: Vec<u8>,
            pub timestamp: chrono::DateTime<chrono::Utc>,
            pub slot_number: u64,
        }
        
        #[derive(Debug, Clone, Serialize, Deserialize)]
        enum KeyServiceKeyUpdate {
            NewKey(KeyServiceEncryptionKey),
            RotateKey {
                old_id: String,
                new_key: KeyServiceEncryptionKey,
            },
            RevokeKey(String),
        }
        
        // Try to deserialize using the key service format
        match bincode::deserialize::<KeyServiceKeyUpdate>(data) {
            Ok(key_service_update) => {
                info!("📋 PARSED KEY SERVICE: Successfully deserialized key service format");
                
                // Convert to our internal format
                let internal_update = match key_service_update {
                    KeyServiceKeyUpdate::NewKey(key) => {
                        info!("📋 CONVERTING: NewKey '{}' with slot {}", key.id, key.slot_number);
                        KeyUpdate::NewKey(EncryptionKey {
                            id: key.id,
                            material: key.key_data,
                            slot_number: Some(key.slot_number),
                            timestamp: Some(key.timestamp),
                        })
                    }
                    KeyServiceKeyUpdate::RotateKey { old_id, new_key } => {
                        info!("📋 CONVERTING: RotateKey {} -> '{}' with slot {}", 
                              old_id, new_key.id, new_key.slot_number);
                        KeyUpdate::RotateKey {
                            old_id,
                            new_key: EncryptionKey {
                                id: new_key.id,
                                material: new_key.key_data,
                                slot_number: Some(new_key.slot_number),
                                timestamp: Some(new_key.timestamp),
                            },
                        }
                    }
                    KeyServiceKeyUpdate::RevokeKey(key_id) => {
                        info!("📋 CONVERTING: RevokeKey '{}'", key_id);
                        KeyUpdate::RevokeKey(key_id)
                    }
                };
                
                Ok(internal_update)
            }
            Err(key_service_err) => {
                debug!("Key service format failed: {}", key_service_err);
                
                // Fall back to our internal format
                match bincode::deserialize::<KeyUpdate>(data) {
                    Ok(internal_update) => {
                        info!("📋 PARSED INTERNAL: Successfully deserialized internal format");
                        Ok(internal_update)
                    }
                    Err(internal_err) => {
                        Err(format!(
                            "Failed both key service format ({}) and internal format ({})", 
                            key_service_err, internal_err
                        ))
                    }
                }
            }
        }
    }

    async fn handle_key_connection(
        mut stream: UnixStream, 
        cache: KeyCache
    ) -> Result<(), EncryptionError> {
        use tokio::io::AsyncReadExt;
        
        info!("New unix socket connection established for key updates");
        let mut buffer = vec![0u8; 4096];
        
        while let Ok(n) = stream.read(&mut buffer).await {
            if n == 0 { 
                info!("Unix socket connection closed by client");
                break; 
            }
            
            info!("Received {} bytes on unix socket", n);
            debug!("Raw data: {:?}", &buffer[..n]);
            
            // Try to deserialize the key service bincode format
            let key_update_result = Self::deserialize_key_service_message(&buffer[..n]);

            match key_update_result {
                Ok(key_update) => {
                    info!("Successfully deserialized KeyUpdate message");
                    match key_update {
                        KeyUpdate::NewKey(key) => {
                            info!("📨 KEY RECEIVED: NEW encryption key '{}' ({} bytes)", key.id, key.material.len());
                            debug!("📨 KEY RECEIVED: Key material hash: {}", hex::encode(&key.material[..8.min(key.material.len())]));
                            
                            // Check if this key has a slot_number for scheduling
                            if let Some(target_slot) = key.slot_number {
                                let current_slot = cache.get_current_slot();
                                let slots_until_activation = target_slot.saturating_sub(current_slot);
                                
                                info!("🔑 SLOT-BASED ACTIVATION: Key '{}' will activate at slot {} ({} slots from now)", 
                                      key.id, target_slot, slots_until_activation);
                                
                                // Convert to scheduled key
                                let scheduled_key = SlotScheduledKey {
                                    key: EncryptionKey {
                                        id: key.id.clone(),
                                        material: key.material.clone(),
                                        slot_number: None, // Clear slot_number since it's now in the scheduled wrapper
                                        timestamp: key.timestamp,
                                    },
                                    activate_at_slot: target_slot,
                                };
                                cache.schedule_key(scheduled_key);
                                info!("✅ KEY SCHEDULED: Key '{}' scheduled for slot {}", key.id, target_slot);
                            } else {
                                // No slot number provided - activate immediately
                                let current_key = cache.get_current_key();
                                if current_key.is_none() {
                                    info!("🔑 GENESIS ACTIVATION: No current key exists, activating '{}' immediately", key.id);
                                } else {
                                    warn!("⚠️  IMMEDIATE ACTIVATION: NewKey '{}' has no slot_number, activating immediately!", key.id);
                                }
                                
                                let clean_key = EncryptionKey {
                                    id: key.id.clone(),
                                    material: key.material.clone(),
                                    slot_number: None,
                                    timestamp: key.timestamp,
                                };
                                cache.update_current_key(clean_key);
                                info!("✅ KEY UPDATED: Current encryption key set to '{}'", key.id);
                            }
                        }
                        KeyUpdate::ScheduledKey(scheduled_key) => {
                            info!("📨 KEY RECEIVED: SCHEDULED encryption key '{}' ({} bytes) -> activate at slot {}", 
                                  scheduled_key.key.id, scheduled_key.key.material.len(), scheduled_key.activate_at_slot);
                            debug!("📨 KEY RECEIVED: Scheduled key material hash: {}", 
                                   hex::encode(&scheduled_key.key.material[..8.min(scheduled_key.key.material.len())]));
                            let current_slot = cache.get_current_slot();
                            let slots_until_activation = scheduled_key.activate_at_slot.saturating_sub(current_slot);
                            cache.schedule_key(scheduled_key.clone());
                            info!("✅ KEY SCHEDULED: Key '{}' scheduled for slot {} ({} slots from now)", 
                                  scheduled_key.key.id, scheduled_key.activate_at_slot, slots_until_activation);
                        }
                        KeyUpdate::RotateKey { old_id, new_key } => {
                            info!("📨 KEY RECEIVED: ROTATION {} -> '{}' ({} bytes)", old_id, new_key.id, new_key.material.len());
                            debug!("📨 KEY RECEIVED: New key material hash: {}", 
                                   hex::encode(&new_key.material[..8.min(new_key.material.len())]));
                            cache.rotate_key(old_id.clone(), new_key.clone());
                            info!("✅ KEY ROTATED: Replaced key '{}' with '{}'", old_id, new_key.id);
                        }
                        KeyUpdate::RevokeKey(key_id) => {
                            warn!("📨 KEY RECEIVED: REVOKE key '{}'", key_id);
                            cache.revoke_key(key_id.clone());
                            warn!("❌ KEY REVOKED: Key '{}' removed from cache", key_id);
                        }
                    }
                }
                Err(e) => {
                    error!("❌ Failed to deserialize KeyUpdate message from {} bytes: {}", n, e);
                    error!("Raw data hex: {}", hex::encode(&buffer[..n]));
                    if let Ok(json_str) = std::str::from_utf8(&buffer[..n]) {
                        error!("Raw data as string: {}", json_str);
                    }
                    return Err(EncryptionError::EncryptionFailed(format!("Key update deserialization failed: {e}")));
                }
            }
        }
        
        info!("Unix socket key connection handler exiting");
        Ok(())
    }
}

impl EncryptionLayer {
    /// Get the appropriate key for a specific slot number
    /// This ensures sequencer and STF use the same key for the same slot
    pub fn get_key_for_slot(&self, slot_number: u64) -> Option<EncryptionKey> {
        // First activate any keys that should be active for this slot
        self.key_cache.check_for_activation(slot_number);
        
        // Then return the current key
        let key = self.key_cache.get_current_key();
        if let Some(ref k) = key {
            info!("🔑 SLOT KEY: Using key '{}' for slot {}", k.id, slot_number);
        } else {
            warn!("🔑 SLOT KEY: No key available for slot {}", slot_number);
        }
        key
    }

    /// Set the current slot number and proactively activate keys for the next slot
    /// This ensures keys are ready before encryption/decryption operations
    pub fn set_current_slot(&self, slot_number: u64) {
        let previous_slot = self.key_cache.get_current_slot();
        
        debug!("⏰ SLOT UPDATE: Setting current slot to {} (was {})", slot_number, previous_slot);
        *self.key_cache.current_slot.write().unwrap() = slot_number;
        
        // Proactively check if we need to activate keys for the next slot (slot_number + 1)
        // This ensures keys are ready before encryption happens
        if slot_number > previous_slot {
            let next_slot = slot_number + 1;
            debug!("🔍 PROACTIVE CHECK: Looking for keys to activate for upcoming slot {}", next_slot);
            let activated_keys = self.key_cache.check_for_activation(next_slot);
            if !activated_keys.is_empty() {
                info!("🔑 PROACTIVE ACTIVATION: {} key(s) pre-activated for slot {} (current slot: {})", 
                      activated_keys.len(), next_slot, slot_number);
                for key in &activated_keys {
                    info!("🔑 PROACTIVE ACTIVATION: Key '{}' is now ready for slot {}", key.id, next_slot);
                }
            } else {
                debug!("🔍 PROACTIVE CHECK: No keys needed activation for slot {}", next_slot);
            }
        }
    }
    
    /// Check if any scheduled keys should be activated (legacy method for compatibility)
    fn check_and_activate_keys(&self) {
        // This should now be a no-op since keys are proactively activated
        // But we keep it for safety in case of edge cases
        let current_slot = self.key_cache.get_current_slot();
        let activated_keys = self.key_cache.update_slot(current_slot);
        if !activated_keys.is_empty() {
            info!("🔑 Fallback activation: {} key(s) activated at slot {} during encrypt/decrypt", 
                  activated_keys.len(), current_slot);
        }
    }
    
    /// Get the current slot number
    pub fn get_current_slot(&self) -> u64 {
        self.key_cache.get_current_slot()
    }
    
    /// Get the current active key
    pub fn get_current_key(&self) -> Option<EncryptionKey> {
        self.key_cache.get_current_key()
    }
    
    /// Update the current slot and activate any scheduled keys (legacy method for manual activation)
    pub fn update_slot(&self, slot_number: u64) -> Vec<EncryptionKey> {
        let activated_keys = self.key_cache.update_slot(slot_number);
        if !activated_keys.is_empty() {
            info!("🎯 Slot {} reached - activated {} key(s)", slot_number, activated_keys.len());
        }
        activated_keys
    }
}

impl EncryptionLayerTrait for EncryptionLayer {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        let current_slot = self.key_cache.get_current_slot();
        
        // Check for key activation before encryption
        self.check_and_activate_keys();
        
        let key = self.key_cache.get_current_key()
            .ok_or(EncryptionError::InvalidKeyFormat("No encryption key available".to_string()))?;
        
        let scheduled_count = self.key_cache.scheduled_keys.read().unwrap().len();
        info!("🔐 ENCRYPT: Using key '{}' at slot {} for {} bytes ({} keys scheduled)", 
              key.id, current_slot, plaintext.len(), scheduled_count);
        debug!("🔐 ENCRYPT: Key material hash: {}", 
               hex::encode(&key.material[..8.min(key.material.len())]));
        
        let result = self.encrypt_with_key(&key.material, plaintext);
        if result.is_ok() {
            info!("✅ ENCRYPT SUCCESS: Key '{}' encrypted {} bytes -> {} bytes at slot {}", 
                  key.id, plaintext.len(), result.as_ref().unwrap().len(), current_slot);
        }
        result
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        let current_slot = self.key_cache.get_current_slot();
        
        // Check for key activation before decryption
        self.check_and_activate_keys();
        
        let key = self.key_cache.get_current_key()
            .ok_or(EncryptionError::InvalidKeyFormat("No key available for decryption".to_string()))?;
        
        let scheduled_count = self.key_cache.scheduled_keys.read().unwrap().len();
        info!("🔓 DECRYPT: Using key '{}' at slot {} for {} bytes ({} keys scheduled)", 
              key.id, current_slot, ciphertext.len(), scheduled_count);
        debug!("🔓 DECRYPT: Key material hash: {}", 
               hex::encode(&key.material[..8.min(key.material.len())]));
        
        let result = self.decrypt_with_key(&key.material, ciphertext);
        match &result {
            Ok(plaintext) => {
                info!("✅ DECRYPT SUCCESS: Key '{}' decrypted {} bytes -> {} bytes at slot {}", 
                      key.id, ciphertext.len(), plaintext.len(), current_slot);
            }
            Err(e) => {
                error!("❌ DECRYPT FAILED: Key '{}' failed to decrypt {} bytes at slot {}: {}", 
                       key.id, ciphertext.len(), current_slot, e);
                error!("❌ DECRYPT DEBUG: This usually means data was encrypted with a different key");
            }
        }
        result
    }

    fn encryption_type(&self) -> &'static str {
        "AES-256-GCM"
    }
}