use std::fmt::Debug;
use std::sync::Arc;

use sov_encryption::{create_encryption_layer, EncryptionLayerTrait};
use sov_rollup_interface::node::da::DaService;


/// A generic wrapper that adds encryption to any DA service.
/// 
/// This wrapper implements the same [`DaService`] interface as the inner service,
/// but automatically encrypts data before sending to the DA layer and decrypts
/// data when reading from the DA layer.
/// 
/// # Type Parameters
/// 
/// * `T` - Must implement [`DaService`]. The inner DA service that will be wrapped with encryption.
#[derive(Clone, Debug)]
pub struct EncryptedDaService<T>
where
    T: DaService,
{
    inner: T,
    encryption: Arc<Box<dyn EncryptionLayerTrait>>,
}

impl<T> EncryptedDaService<T> 
where
    T: DaService,
{
    /// Create a new encrypted DA service wrapper
    pub fn new(inner: T, encryption_config: sov_encryption::EncryptionConfig) -> Self {
        tracing::info!("=== CREATING ENCRYPTED DA SERVICE ===");
        tracing::info!("Encryption cipher: {:?}", encryption_config.cipher_type);
        
        // Create the encryption layer from config
        let encryption = Arc::new(create_encryption_layer(encryption_config));
        
        Self {
            inner,
            encryption,
        }
    }

    /// Create with a custom encryption layer
    pub fn new_with_encryption_layer(inner: T, encryption: Box<dyn EncryptionLayerTrait>) -> Self {
        Self {
            inner,
            encryption: Arc::new(encryption),
        }
    }

    /// Get reference to the inner DA service
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Get mutable reference to the inner DA service
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Extract the inner DA service
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Get reference to the encryption layer
    pub fn encryption(&self) -> &Arc<Box<dyn EncryptionLayerTrait>> {
        &self.encryption
    }

}



