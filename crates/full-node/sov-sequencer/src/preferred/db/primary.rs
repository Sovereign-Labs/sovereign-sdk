use std::{collections::VecDeque, num::NonZero, path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_full_node_configs::sequencer::PostgresConfig;
use sov_modules_api::{FullyBakedTx, TxHash, VisibleSlotNumber};
use tokio::sync::watch;

use crate::preferred::{
    db::{
        postgres::PostgresBackend, rocksdb::RocksDbBackend, BatchToStore, DbSnapshotData,
        PreferredSequencerCache, PreferredSequencerDb, PreferredSequencerDbBackend,
    },
    exit_rollup,
};

// Don’t prune the data from the database immediately — give the replica some time to read it before it is pruned.
const PRUNING_LAG: u64 = 10;

/// Primary sequencer database implementation with real storage backend.
pub struct PrimarySequencerDb {
    backend: Box<dyn PreferredSequencerDbBackend>,
    shutdown_sender: watch::Sender<()>,
}

impl PrimarySequencerDb {
    pub(crate) async fn new(
        shutdown_sender: watch::Sender<()>,
        storage_path: &Path,
        postgres_config: &Option<PostgresConfig>,
    ) -> Result<Self> {
        let backend: Box<dyn PreferredSequencerDbBackend> = {
            if let Some(postgres_config) = &postgres_config {
                Box::new(PostgresBackend::connect(postgres_config).await?)
            } else {
                Box::new(RocksDbBackend::new(storage_path).await?)
            }
        };

        Ok(Self {
            backend,
            shutdown_sender,
        })
    }

    async fn debug_assert_in_progress_batch_is_none(
        msg: &str,
        backend: &mut Box<dyn PreferredSequencerDbBackend>,
        shutdown_sender: &watch::Sender<()>,
    ) {
        if cfg!(debug_assertions) {
            match backend.read_in_progress_batch().await {
                Ok(None) => {}
                other => {
                    tracing::error!("{msg}: {other:?}");
                    exit_rollup(shutdown_sender).await;
                }
            }
        }
    }
}

#[async_trait]
impl PreferredSequencerDb for PrimarySequencerDb {
    async fn initial_data(&self) -> Result<(SequenceNumber, PreferredSequencerCache)> {
        let DbSnapshotData {
            completed_blobs,
            in_progress_batch,
        } = self.backend.current_data().await?;

        let completed_blobs = VecDeque::from(completed_blobs);

        let sequence_number_of_next_blob = match (completed_blobs.back(), &in_progress_batch) {
            (Some(blob), None) => blob.sequence_number() + 1,
            (None, Some(batch)) => batch.sequence_number + 1,
            (Some(blob), Some(batch)) => {
                std::cmp::max(blob.sequence_number(), batch.sequence_number) + 1
            }
            (None, None) => 0,
        };

        Ok((
            sequence_number_of_next_blob,
            PreferredSequencerCache::new(
                completed_blobs,
                in_progress_batch,
                self.shutdown_sender.clone(),
            ),
        ))
    }

    #[tracing::instrument(skip_all, level = "info")]
    async fn bulk_insert_txs(
        &mut self,
        txs: Vec<(FullyBakedTx, TxHash)>,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
    ) -> Result<()> {
        self.backend
            .batch_add_txs(sequence_number, tx_idx_within_batch, &txs)
            .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    async fn start_batch(
        &mut self,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
    ) -> Result<SequenceNumber> {
        Self::debug_assert_in_progress_batch_is_none(
            "Cached in-progress batch state (None) didn't match backend db state",
            &mut self.backend,
            &self.shutdown_sender,
        )
        .await;

        tracing::debug!(
            sequence_number,
            blob_id,
            %visible_slot_number_after_increase,
            visible_slots_to_advance,
            "Storing new rollup block"
        );

        let batch_to_store = BatchToStore {
            sequence_number,
            blob_id,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        };
        self.backend.begin_rollup_block(batch_to_store).await?;

        Ok(sequence_number)
    }

    #[tracing::instrument(skip_all, level = "info")]
    async fn insert_proof_blob(
        &mut self,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
        sequence_number: SequenceNumber,
    ) -> Result<SequenceNumber> {
        self.backend
            .add_proof_blob(sequence_number, blob_id, data.clone())
            .await?;
        Ok(sequence_number)
    }

    #[tracing::instrument(skip_all, level = "info")]
    async fn terminate_batch(&mut self, batch: BatchToStore) -> Result<()> {
        self.backend.end_rollup_block(batch).await?;
        Self::debug_assert_in_progress_batch_is_none(
            "Backend didn't remove in-progress batch from database when ending rollup block",
            &mut self.backend,
            &self.shutdown_sender,
        )
        .await;
        Ok(())
    }

    #[tracing::instrument(skip_all, level = "info")]
    async fn prune_db(&mut self, prune_up_to_including: SequenceNumber) -> Result<()> {
        if let Some(prune_up_to_including) = prune_up_to_including.checked_sub(PRUNING_LAG) {
            self.backend.prune(prune_up_to_including).await?;
        }
        Ok(())
    }
}
