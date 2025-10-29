use sov_blob_sender::BlobExecutionStatus;
use sov_blob_sender::{BlobInternalId, BlobSender, BlobToSend};
use sov_blob_storage::{EncryptedPreferredBatchData, PreferredBatchData, PreferredProofData};
use sov_db::ledger_db::LedgerDb;
use sov_encryption::{EncryptionLayer, EncryptionLayerTrait};
use sov_modules_api::TxHash;
use sov_rollup_interface::node::da::DaService;
use std::{
    path::Path,
    sync::{atomic::AtomicUsize, Arc},
};
use tokio::sync::broadcast;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::debug;

use super::db::{PreferredSequencerReadBatch, PreferredSequencerReadBlob};
use crate::{common::TxStatusBlobSenderHooks, TxStatusManager};

/// Wrapper around [`BlobSender`] with preferred blob -specific logic.
pub struct PreferredBlobSender<Da: DaService> {
    inner: Option<BlobSender<Da, TxStatusBlobSenderHooks<Da::Spec>, LedgerDb>>,
    nb_of_concurrent_blob_submissions: Arc<AtomicUsize>,
    encryption_layer: Option<EncryptionLayer>,
}

impl<Da: DaService> PreferredBlobSender<Da> {
    pub(crate) async fn new(
        da: Da,
        ledger_db: LedgerDb,
        all_completed_blobs: Vec<PreferredSequencerReadBlob>,
        storage_path: Box<Path>,
        tx_status_manager: TxStatusManager<Da::Spec>,
        shutdown_sender: watch::Sender<()>,
        blob_processing_timeout: Duration,
        blobs_sender_channel: broadcast::Sender<BlobExecutionStatus<Da::Spec>>,
        is_replica: bool,
        encryption_config: Option<sov_encryption::EncryptionConfig>,
    ) -> anyhow::Result<(Self, Option<JoinHandle<()>>)> {
        let nb_of_concurrent_blob_submissions = Arc::new(AtomicUsize::new(0));
        
        // Initialize encryption layer if config is provided
        // The layer will automatically handle its own key management based on config
        let encryption_layer = if let Some(config) = encryption_config {
            Some(EncryptionLayer::new(config).await?)
        } else {
            None
        };
        
        if is_replica {
            Ok((
                Self {
                    inner: None,
                    nb_of_concurrent_blob_submissions,
                    encryption_layer,
                },
                None,
            ))
        } else {
            // It's possible that sov-blob-sender's DB might miss some blob data at
            // node startup due to:
            //  1. Disk failure (the sequencer can use Postgres so it's durable).
            //  2. DB corruption.
            //  3. Node crash at an inconvenient time.
            // Let's restore all missing blob data to make sure they land on the DA.
            let blobs_to_send = create_blobs_to_send(all_completed_blobs, encryption_layer.as_ref())?;
            let (inner, blob_sender_handle) = BlobSender::new(
                da.clone(),
                ledger_db,
                storage_path.as_ref(),
                TxStatusBlobSenderHooks::new(tx_status_manager.clone()),
                shutdown_sender,
                blob_processing_timeout,
                Some(blobs_sender_channel),
                blobs_to_send,
                nb_of_concurrent_blob_submissions.clone(),
            )
            .await?;

            Ok((
                Self {
                    inner: Some(inner),
                    nb_of_concurrent_blob_submissions,
                    encryption_layer,
                },
                Some(blob_sender_handle),
            ))
        }
    }

    pub(crate) async fn publish_proof(
        &mut self,
        proof_data: Arc<[u8]>,
        sequence_number: u64,
        blob_id: BlobInternalId,
    ) -> anyhow::Result<()> {
        let Some(ref mut inner) = self.inner else {
            return Ok(());
        };

        let blob_bytes = proof_bytes(&proof_data, sequence_number)?;

        debug!(
            sequence_number,
            blob_id, "Dispatching proof blob for publishing"
        );

        inner.publish_proof_blob(blob_bytes, blob_id).await?;

        Ok(())
    }

    pub(crate) async fn publish_batch(
        &mut self,
        batch: PreferredSequencerReadBatch,
    ) -> anyhow::Result<()> {
        let Some(ref mut inner) = self.inner else {
            return Ok(());
        };

        let blob_id = batch.blob_id;
        let serialized = batch_bytes(batch, self.encryption_layer.as_ref())?;
        let data = Arc::from(serialized);

        inner.publish_batch_blob(data, blob_id).await?;

        Ok(())
    }

    pub async fn publish_blobs_for_recovery(
        &mut self,
        completed_blobs: Vec<PreferredSequencerReadBlob>,
    ) -> anyhow::Result<()> {
        for blob in completed_blobs {
            match blob {
                PreferredSequencerReadBlob::Batch(batch) => {
                    self.publish_batch(batch).await?;
                }
                PreferredSequencerReadBlob::Proof {
                    data,
                    sequence_number,
                    blob_id,
                } => {
                    self.publish_proof(data, sequence_number, blob_id).await?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn nb_of_in_flight_blobs(&self) -> Arc<AtomicUsize> {
        self.nb_of_concurrent_blob_submissions.clone()
    }

    pub(crate) async fn add_txs(&self, blob_id: BlobInternalId, tx_hashes: Arc<Vec<TxHash>>) {
        let Some(ref inner) = self.inner else {
            return;
        };

        inner.hooks().add_txs(blob_id, tx_hashes).await;
    }
}

pub fn create_blobs_to_send(
    completed_blobs: Vec<PreferredSequencerReadBlob>,
    encryption_layer: Option<&EncryptionLayer>,
) -> anyhow::Result<Vec<(BlobToSend, BlobInternalId)>> {
    let mut blobs_to_send = Vec::new();

    for blob in completed_blobs {
        match blob {
            PreferredSequencerReadBlob::Batch(batch) => {
                let blob_id = batch.blob_id;
                let serialized = batch_bytes(batch, encryption_layer)?;
                let data = Arc::from(serialized);
                blobs_to_send.push((BlobToSend::Batch { data }, blob_id));
            }
            PreferredSequencerReadBlob::Proof {
                data,
                sequence_number,
                blob_id,
            } => {
                let data = proof_bytes(&data, sequence_number)?;
                debug!(
                    sequence_number,
                    blob_id, "Dispatching proof blob for publishing"
                );

                blobs_to_send.push((BlobToSend::Proof { data }, blob_id));
            }
        }
    }

    Ok(blobs_to_send)
}

fn proof_bytes(proof_data: &[u8], sequence_number: u64) -> anyhow::Result<Arc<[u8]>> {
    let blob = PreferredProofData {
        sequence_number,
        data: proof_data.to_vec(),
    };
    Ok(Arc::from(borsh::to_vec(&blob)?))
}

fn batch_bytes(
    batch: PreferredSequencerReadBatch,
    encryption_layer: Option<&EncryptionLayer>
) -> anyhow::Result<Vec<u8>> {
    if let Some(encryptor) = encryption_layer {
        // Set current slot for proactive key activation during encryption
        // Use the visible slot number from the batch for encryption context
        let slot_number = batch.visible_slot_number_after_increase.as_true().get();
        tracing::info!("📦 SEQUENCER: Setting slot {} for batch encryption (seq #{}, {} txs)", 
                       slot_number, batch.sequence_number, batch.txs.len());
        encryptor.set_current_slot(slot_number);
        
        // Serialize the entire transaction vector
        let txs_serialized = borsh::to_vec(&*batch.txs)?;
        
        // Encrypt the serialized transaction data as one ciphertext
        tracing::info!("🔐 SEQUENCER: Encrypting batch #{} with {} transactions ({} bytes) at slot {}", 
                       batch.sequence_number, batch.txs.len(), txs_serialized.len(), slot_number);
        let encrypted_txs_data = encryptor.encrypt(&txs_serialized)?;
        
        // Create batch with serialized encrypted blob + metadata including tx hashes
        tracing::debug!("📦 Creating encrypted batch with tx hashes");
        borsh::to_vec(&EncryptedPreferredBatchData {
            sequence_number: batch.sequence_number,
            visible_slots_to_advance: batch.visible_slots_to_advance,
            encrypted_txs_data,
            tx_hashes: batch.tx_hashes,
        }).map_err(Into::into)
    } else {
        // Original unencrypted path if encryption is not enabled
        tracing::debug!("📦 Creating batch with unencrypted txs");
        borsh::to_vec(&PreferredBatchData {
            sequence_number: batch.sequence_number,
            visible_slots_to_advance: batch.visible_slots_to_advance,
            data: batch.txs,
        }).map_err(Into::into)
    }
}