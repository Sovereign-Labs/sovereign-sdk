use std::sync::Arc;

use sov_encryption::EncryptionLayerTrait;
use sov_rollup_interface::da::Time;
use sov_rollup_interface::node::da::SlotData;

/// A wrapper around the inner FilteredBlock for encrypted DA
/// 
/// This implementation uses eager decryption for simplicity and readability.
/// All blob data is decrypted when the block is first processed, and stored
/// for later access via extract_relevant_blobs.
pub struct EncryptedFilteredBlock<T>
where
    T: SlotData,
{
    inner: T,
    encryption: Arc<Box<dyn EncryptionLayerTrait>>,
    // Pre-decrypted blob data for eager decryption
    decrypted_batch_blobs: Vec<Vec<u8>>,
    decrypted_proof_blobs: Vec<Vec<u8>>,
}

// Manual implementation of required traits since we can't derive them for trait objects
impl<T> Clone for EncryptedFilteredBlock<T>
where
    T: SlotData + Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            encryption: self.encryption.clone(),
            decrypted_batch_blobs: self.decrypted_batch_blobs.clone(),
            decrypted_proof_blobs: self.decrypted_proof_blobs.clone(),
        }
    }
}

impl<T> std::fmt::Debug for EncryptedFilteredBlock<T>
where
    T: SlotData + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedFilteredBlock")
            .field("inner", &self.inner)
            .field("encryption", &format!("{:?}", &*self.encryption))
            .field("decrypted_batch_blobs_count", &self.decrypted_batch_blobs.len())
            .field("decrypted_proof_blobs_count", &self.decrypted_proof_blobs.len())
            .finish()
    }
}

impl<T> PartialEq for EncryptedFilteredBlock<T>
where
    T: SlotData + PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner &&
        self.decrypted_batch_blobs == other.decrypted_batch_blobs &&
        self.decrypted_proof_blobs == other.decrypted_proof_blobs
    }
}

// For SlotData requirements, we need Serialize and Deserialize
// We'll serialize the inner type and decrypted blob data
impl<T> serde::Serialize for EncryptedFilteredBlock<T>
where
    T: SlotData + serde::Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("EncryptedFilteredBlock", 3)?;
        state.serialize_field("inner", &self.inner)?;
        state.serialize_field("decrypted_batch_blobs", &self.decrypted_batch_blobs)?;
        state.serialize_field("decrypted_proof_blobs", &self.decrypted_proof_blobs)?;
        state.end()
    }
}

impl<'de, T> serde::Deserialize<'de> for EncryptedFilteredBlock<T>
where
    T: SlotData,
    for<'a> T: serde::Deserialize<'a>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::{Deserialize};
        
        #[derive(Deserialize)]
        struct EncryptedFilteredBlockHelper<T> {
            inner: T,
            decrypted_batch_blobs: Vec<Vec<u8>>,
            decrypted_proof_blobs: Vec<Vec<u8>>,
        }
        
        let helper = EncryptedFilteredBlockHelper::<T>::deserialize(deserializer)?;
        
        // Note: When deserializing, we create a dummy encryption layer
        // This is a limitation of the current design - in practice you'd need to 
        // reconstruct the encryption layer from context
        use sov_encryption::config::{EncryptionConfig, KeyClientConfig, CipherType};
        let dummy_config = EncryptionConfig {
            key_client: KeyClientConfig::Static {
                encryption_key: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                decryption_key: None,
            },
            cipher_type: CipherType::Aes256Gcm,
            key_rotation_interval: None,
        };
        let encryption = Arc::new(sov_encryption::create_encryption_layer(dummy_config));
        
        Ok(Self {
            inner: helper.inner,
            encryption,
            decrypted_batch_blobs: helper.decrypted_batch_blobs,
            decrypted_proof_blobs: helper.decrypted_proof_blobs,
        })
    }
}

impl<T> EncryptedFilteredBlock<T>
where
    T: SlotData,
{
    pub async fn new(inner: T, encryption: Arc<Box<dyn EncryptionLayerTrait>>) -> Result<Self, anyhow::Error> {
        // For backward compatibility - create with empty decrypted blobs
        Ok(Self {
            inner,
            encryption,
            decrypted_batch_blobs: Vec::new(),
            decrypted_proof_blobs: Vec::new(),
        })
    }
    
    pub async fn new_with_decrypted_blobs(
        inner: T, 
        encryption: Arc<Box<dyn EncryptionLayerTrait>>,
        decrypted_batch_blobs: Vec<Vec<u8>>,
        decrypted_proof_blobs: Vec<Vec<u8>>,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            inner,
            encryption,
            decrypted_batch_blobs,
            decrypted_proof_blobs,
        })
    }
    
    pub fn inner(&self) -> &T {
        &self.inner
    }
    
    /// Decrypt a blob - this is a simple helper method for when decryption is needed
    pub async fn decrypt_blob_data(&self, encrypted_data: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
        self.encryption.decrypt(encrypted_data).await
            .map_err(|e| anyhow::anyhow!("Failed to decrypt blob: {}", e))
    }
    
    /// Get the pre-decrypted batch blob data at the given index
    /// This allows applications to access decrypted data directly
    pub fn get_decrypted_batch_blob(&self, index: usize) -> Option<&[u8]> {
        self.decrypted_batch_blobs.get(index).map(|v| v.as_slice())
    }
    
    /// Get the pre-decrypted proof blob data at the given index
    /// This allows applications to access decrypted data directly
    pub fn get_decrypted_proof_blob(&self, index: usize) -> Option<&[u8]> {
        self.decrypted_proof_blobs.get(index).map(|v| v.as_slice())
    }
    
    /// Get all pre-decrypted batch blob data
    pub fn get_decrypted_batch_blobs(&self) -> &[Vec<u8>] {
        &self.decrypted_batch_blobs
    }
    
    /// Get all pre-decrypted proof blob data
    pub fn get_decrypted_proof_blobs(&self) -> &[Vec<u8>] {
        &self.decrypted_proof_blobs
    }
}

impl<T> SlotData for EncryptedFilteredBlock<T>
where
    T: SlotData,
{
    type BlockHeader = T::BlockHeader;

    fn hash(&self) -> [u8; 32] {
        self.inner.hash()
    }

    fn header(&self) -> &Self::BlockHeader {
        self.inner.header()
    }

    fn timestamp(&self) -> Time {
        self.inner.timestamp()
    }
}