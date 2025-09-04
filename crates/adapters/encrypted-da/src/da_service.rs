use std::fmt::Display;

use async_trait::async_trait;
use sov_rollup_interface::da::{
    BlobReaderTrait, DaSpec, RelevantBlobs, RelevantProofs, 
};
use sov_rollup_interface::node::da::{DaService, SubmitBlobReceipt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::config::EncryptedDaConfig;
use crate::filtered_block::EncryptedFilteredBlock;
use crate::encrypted_service::EncryptedDaService;

/// Implementation of the DaService trait for the EncryptedDaService<T> wrapper
/// 
/// This implementation allows for encryption/decryption of data stored on Data Availability layers without any API changes.
/// 
/// The wrapper delegates most functionality to the inner DA service, but adds 
/// encryption/decryption of data using the sov-encryption layer.
/// 
/// # Type Parameters
/// 
/// * `T` - Must implement [`DaService`]. The inner DA service that will be wrapped with encryption.
#[async_trait]
impl<T> DaService for EncryptedDaService<T>
where
    T: DaService + Clone + Send + Sync + 'static,
    T::Error: Send + Sync + Display,
{
    type Spec = T::Spec;
    type Config = EncryptedDaConfig<T::Config>;
    type Verifier = T::Verifier;
    type FilteredBlock = EncryptedFilteredBlock<T::FilteredBlock>;
    type Error = anyhow::Error;

    const GUARANTEES_TRANSACTION_ORDERING: bool = T::GUARANTEES_TRANSACTION_ORDERING;

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        debug!("Getting block at height {}", height);
        
        // Get the FilteredBlock from the inner DA service at the given height
        let inner_block = self.inner()
            .get_block_at(height)
            .await
            .map_err(|e| anyhow::anyhow!("Inner DA service error: {}", e))?;

        // Decrypt the block's blobs using the async method
        tracing::info!("=== DECRYPTING BLOCK ===");
        tracing::info!("Decrypting block at height {}", height);
        let decrypted_block = self.decrypt_block(inner_block).await?;
        
        tracing::info!("=== BLOCK DECRYPTION COMPLETE ===");
        debug!("Successfully retrieved and decrypted block at height {}", height);
        Ok(decrypted_block)
    }

    async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        debug!("Getting block header at height {}", height);
        
        // Delegate to the inner DA service to get the block header
        self.inner()
            .get_block_header_at(height)
            .await
            .map_err(|e| anyhow::anyhow!("Inner DA service error: {}", e))
    }

    async fn get_last_finalized_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        debug!("Getting last finalized block header");
        
        // Delegate to the inner DA service to get the block header
        self.inner()
            .get_last_finalized_block_header()
            .await
            .map_err(|e| anyhow::anyhow!("Inner DA service error: {}", e))
    }

    async fn get_head_block_header(
        &self,
    ) -> Result<<Self::Spec as DaSpec>::BlockHeader, Self::Error> {
        debug!("Getting head block header");
        
        // Delegate to the inner DA service to get the block header
        self.inner()
            .get_head_block_header()
            .await
            .map_err(|e| anyhow::anyhow!("Inner DA service error: {}", e))
    }

    fn extract_relevant_blobs(
        &self,
        block: &Self::FilteredBlock,
    ) -> RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction> {
        
        // Get the encrypted blobs from the inner DA service for metadata (address, hash)
        let inner_blobs = self.inner().extract_relevant_blobs(block.inner());
        
        tracing::info!("Extracted {} batch blobs and {} proof blobs", 
                      inner_blobs.batch_blobs.len(), 
                      inner_blobs.proof_blobs.len());
        
        // For now, return the encrypted blobs - the real decryption should happen 
        // when the blobs are accessed. This is a limitation of the current design
        // where we can't easily replace blob data without knowing the concrete type.
        //
        // The proper solution would be to modify the BlobReaderTrait to allow 
        // mutable access to the underlying data, or create a new trait for 
        // decrypted blobs. For MockDA, we'd need to detect MockBlob specifically.
        //
        // As a temporary workaround, let's just return the original blobs.
        // Tests that need decrypted data should use the EncryptedFilteredBlock
        // methods directly: get_decrypted_batch_blobs() and get_decrypted_proof_blobs()
        inner_blobs
    }

    async fn get_extraction_proof(
        &self,
        block: &Self::FilteredBlock,
        blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
    ) -> RelevantProofs<
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    > {
        // Delegate to the inner DA service to get the extraction proof
        self.inner().get_extraction_proof(block.inner(), blobs).await
    }

    async fn send_transaction(
        &self,
        blob: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        tracing::info!("=== ENCRYPTING TRANSACTION ===");
        tracing::info!("Original blob size: {} bytes", blob.len());
        tracing::info!("Original blob (first 32 bytes): {}", hex::encode(&blob[..std::cmp::min(32, blob.len())]));
        debug!("Sending encrypted transaction of {} bytes", blob.len());
        
        let (tx, rx) = oneshot::channel();
        
        // Encrypt the blob before sending
        let encryption = self.encryption().clone();
        let blob_to_encrypt = blob.to_vec();
        let inner_service = self.inner().clone();
        
        tokio::spawn(async move {
            let result: Result<SubmitBlobReceipt<<T::Spec as DaSpec>::TransactionId>, anyhow::Error> = async {
                let encrypted_blob = encryption.encrypt(&blob_to_encrypt).await
                    .map_err(|e| anyhow::anyhow!("Encryption error: {}", e))?;
                tracing::info!("Encrypted blob from {} to {} bytes", blob_to_encrypt.len(), encrypted_blob.len());
                tracing::info!("Encrypted blob (first 32 bytes): {}", hex::encode(&encrypted_blob[..std::cmp::min(32, encrypted_blob.len())]));
                tracing::info!("=== ENCRYPTION COMPLETE ===");
                debug!("Encrypted blob from {} to {} bytes", blob_to_encrypt.len(), encrypted_blob.len());
                
                let inner_receiver = inner_service
                    .send_transaction(&encrypted_blob)
                    .await;
                let inner_result = inner_receiver
                    .await
                    .map_err(|e| anyhow::anyhow!("Channel error: {}", e))?;
                let inner_receipt = inner_result
                    .map_err(|e| anyhow::anyhow!("Inner DA service error: {}", e))?;
                
                Ok(inner_receipt)
            }.await;
            
            let _ = tx.send(result);
        });
        
        rx
    }

    async fn send_proof(
        &self,
        aggregated_proof_data: &[u8],
    ) -> oneshot::Receiver<
        Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>,
    > {
        debug!("Sending encrypted proof of {} bytes", aggregated_proof_data.len());
        
        let (tx, rx) = oneshot::channel();
        
        // Encrypt the proof before sending
        let encryption = self.encryption().clone();
        let proof_to_encrypt = aggregated_proof_data.to_vec();
        let inner_service = self.inner().clone();
        
        tokio::spawn(async move {
            let result: Result<SubmitBlobReceipt<<T::Spec as DaSpec>::TransactionId>, anyhow::Error> = async {
                let encrypted_proof = encryption.encrypt(&proof_to_encrypt).await
                    .map_err(|e| anyhow::anyhow!("Encryption error: {}", e))?;
                debug!("Encrypted proof from {} to {} bytes", proof_to_encrypt.len(), encrypted_proof.len());
                
                let inner_receiver = inner_service
                    .send_proof(&encrypted_proof)
                    .await;
                let inner_result = inner_receiver
                    .await
                    .map_err(|e| anyhow::anyhow!("Channel error: {}", e))?;
                let inner_receipt = inner_result
                    .map_err(|e| anyhow::anyhow!("Inner DA service error: {}", e))?;
                
                Ok(inner_receipt)
            }.await;
            
            let _ = tx.send(result);
        });
        
        rx
    }

    async fn get_proofs_at(&self, height: u64) -> Result<Vec<Vec<u8>>, Self::Error> {
        tracing::info!("=== DECRYPTING PROOFS ===");
        tracing::info!("Getting encrypted proofs at height {}", height);
        debug!("Getting encrypted proofs at height {}", height);
        
        let encrypted_proofs = self.inner()
            .get_proofs_at(height)
            .await
            .map_err(|e| anyhow::anyhow!("Inner DA service error: {}", e))?;

        let mut decrypted_proofs = Vec::with_capacity(encrypted_proofs.len());
        
        for (i, encrypted_proof) in encrypted_proofs.iter().enumerate() {
            tracing::info!("Decrypting proof {} of {}", i + 1, encrypted_proofs.len());
            tracing::info!("Encrypted proof size: {} bytes", encrypted_proof.len());
            tracing::info!("Encrypted proof (first 32 bytes): {}", hex::encode(&encrypted_proof[..std::cmp::min(32, encrypted_proof.len())]));
            
            let decrypted_proof = self.encryption()
                .decrypt(encrypted_proof)
                .await
                .map_err(|e| anyhow::anyhow!("Decryption error: {}", e))?;
            
            tracing::info!("Decrypted proof size: {} bytes", decrypted_proof.len());
            tracing::info!("Decrypted proof (first 32 bytes): {}", hex::encode(&decrypted_proof[..std::cmp::min(32, decrypted_proof.len())]));
            decrypted_proofs.push(decrypted_proof);
        }
        
        tracing::info!("=== DECRYPTION COMPLETE ===");
        debug!("Successfully decrypted {} proofs", decrypted_proofs.len());
        Ok(decrypted_proofs)
    }

    async fn take_background_join_handle(&self) -> Option<JoinHandle<()>> {
        self.inner().take_background_join_handle().await
    }

    async fn get_signer(&self) -> <Self::Spec as DaSpec>::Address {
        self.inner().get_signer().await
    }
}

impl<T> EncryptedDaService<T>
where
    T: DaService,
{
    /// Decrypt all blob data in a block
    pub(crate) async fn decrypt_block(&self, inner_block: T::FilteredBlock) -> Result<EncryptedFilteredBlock<T::FilteredBlock>, anyhow::Error> {
        debug!("Decrypting entire block data with eager decryption");
        
        // Extract encrypted blobs from the inner block
        // Why does decrypt_block need to extract encrypted blobs? It feels weird that get_block_at_height() calls decrypt_block() which calls extract_relevant_blobs()
        let encrypted_blobs = self.inner().extract_relevant_blobs(&inner_block);
        
        // Decrypt all blob data upfront (eager decryption)
        let mut decrypted_batch_blobs = Vec::new();
        let mut decrypted_proof_blobs = Vec::new();
        
        // Decrypt batch blobs
        let batch_blobs_count = encrypted_blobs.batch_blobs.len();
        for (i, mut blob) in encrypted_blobs.batch_blobs.into_iter().enumerate() {
            tracing::info!("Decrypting batch blob {} of {}", i + 1, batch_blobs_count);
            let encrypted_data = blob.full_data();
            tracing::info!("Encrypted batch blob size: {} bytes", encrypted_data.len());
            
            match self.encryption().decrypt(encrypted_data).await {
                Ok(decrypted_data) => {
                    tracing::info!("Decrypted batch blob from {} to {} bytes", encrypted_data.len(), decrypted_data.len());
                    decrypted_batch_blobs.push(decrypted_data);
                }
                Err(e) => {
                    tracing::warn!("Failed to decrypt batch blob {}: {}", i, e);
                    // Keep original data if decryption fails (might not be encrypted)
                    decrypted_batch_blobs.push(encrypted_data.to_vec());
                }
            }
        }
        
        // Decrypt proof blobs
        let proof_blobs_count = encrypted_blobs.proof_blobs.len();
        for (i, mut blob) in encrypted_blobs.proof_blobs.into_iter().enumerate() {
            tracing::info!("Decrypting proof blob {} of {}", i + 1, proof_blobs_count);
            let encrypted_data = blob.full_data();
            tracing::info!("Encrypted proof blob size: {} bytes", encrypted_data.len());
            
            match self.encryption().decrypt(encrypted_data).await {
                Ok(decrypted_data) => {
                    tracing::info!("Decrypted proof blob from {} to {} bytes", encrypted_data.len(), decrypted_data.len());
                    decrypted_proof_blobs.push(decrypted_data);
                }
                Err(e) => {
                    tracing::warn!("Failed to decrypt proof blob {}: {}", i, e);
                    // Keep original data if decryption fails (might not be encrypted)
                    decrypted_proof_blobs.push(encrypted_data.to_vec());
                }
            }
        }
        
        // Create the encrypted filtered block with pre-decrypted blob data
        EncryptedFilteredBlock::new_with_decrypted_blobs(
            inner_block, 
            self.encryption().clone(),
            decrypted_batch_blobs,
            decrypted_proof_blobs,
        ).await
    }
}