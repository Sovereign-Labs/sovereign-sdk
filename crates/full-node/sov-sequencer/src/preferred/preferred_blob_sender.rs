use sov_blob_sender::BlobExecutionStatus;
use sov_blob_sender::{BlobInternalId, BlobSender, BlobToSend};
use sov_blob_storage::{PreferredBatchData, PreferredProofData};
use sov_db::ledger_db::LedgerDb;
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
    ) -> anyhow::Result<(Self, Option<JoinHandle<()>>)> {
        let nb_of_concurrent_blob_submissions = Arc::new(AtomicUsize::new(0));
        if is_replica {
            Ok((
                Self {
                    inner: None,
                    nb_of_concurrent_blob_submissions,
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
            let blobs_to_send = create_blobs_to_send(all_completed_blobs)?;
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
        let data = batch_bytes(batch)?;

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
) -> anyhow::Result<Vec<(BlobToSend, BlobInternalId)>> {
    let mut blobs_to_send = Vec::new();

    for blob in completed_blobs {
        match blob {
            PreferredSequencerReadBlob::Batch(batch) => {
                let blob_id = batch.blob_id;
                let data = batch_bytes(batch)?;
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

fn batch_bytes(batch: PreferredSequencerReadBatch) -> anyhow::Result<Arc<[u8]>> {
    Ok(borsh::to_vec::<PreferredBatchData>(&PreferredBatchData {
        sequence_number: batch.sequence_number,
        visible_slots_to_advance: batch.visible_slots_to_advance,
        data: batch.txs,
    })?
    .into())
}
